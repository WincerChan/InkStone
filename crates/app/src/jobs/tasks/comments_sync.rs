use std::collections::HashMap;

use chrono::{DateTime, Utc};
use tracing::{info, warn};

use crate::config::AppConfig;
use crate::jobs::JobError;
use crate::jobs::tasks::feed_index::{SearchIndexEntry, parse_search_index_entries};
use crate::state::AppState;
use inkstone_core::types::slug::Slug;
use inkstone_infra::db::{
    CommentRecord, DiscussionRecord, find_discussion_by_discussion_id, find_discussion_by_post_id,
    list_discussions, replace_comments, upsert_discussion,
};
use inkstone_infra::github::{DiscussionInfo, GithubAppClient};

#[derive(Debug, Default)]
pub struct CommentsSyncStats {
    pub posts_seen: usize,
    pub discussions_created: usize,
    pub discussions_synced: usize,
    pub errors: usize,
}

#[derive(Debug)]
struct PostRef {
    post_id: String,
    slug: String,
    title: String,
    summary: Option<String>,
}

#[derive(Debug)]
struct CommentsConfig {
    app_id: u64,
    installation_id: u64,
    private_key: String,
    repo_owner: String,
    repo_name: String,
    discussion_category_id: Option<String>,
}

pub async fn run(state: &AppState, rebuild: bool) -> Result<CommentsSyncStats, JobError> {
    {
        let mut health = state.admin_health.lock().await;
        health.comments_sync_last_run = Some(Utc::now());
    }
    let Some(pool) = state.db.as_ref() else {
        return Err(JobError::Comments("db not configured".to_string()));
    };
    let Some(config) = CommentsConfig::from_app(&state.config) else {
        return Ok(CommentsSyncStats::default());
    };
    info!(
        app_id = config.app_id,
        installation_id = config.installation_id,
        repo_owner = %config.repo_owner,
        repo_name = %config.repo_name,
        "comments sync configured"
    );
    let client = GithubAppClient::new(
        state.http_client.clone(),
        config.app_id,
        config.installation_id,
        config.private_key.clone(),
    );

    let mut stats = CommentsSyncStats::default();
    let posts = fetch_posts(state).await?;
    stats.posts_seen = posts.len();

    for post in posts {
        match ensure_discussion_for_post(&client, pool, &config, &post).await {
            Ok(outcome) => {
                if outcome.created {
                    stats.discussions_created += 1;
                }
            }
            Err(err) => {
                stats.errors += 1;
                warn!(error = %err, slug = %post.slug, "failed to ensure discussion");
            }
        }
    }

    let discussions = list_discussions(pool).await?;
    let latest_updates = if rebuild {
        HashMap::new()
    } else {
        match fetch_discussion_updates(&client, &discussions).await {
            Ok(updates) => updates,
            Err(err) => {
                warn!(error = %err, "discussion precheck failed; syncing all");
                HashMap::new()
            }
        }
    };
    for discussion in discussions {
        if !is_github_discussion_id(&discussion.discussion_id) {
            continue;
        }
        if !rebuild && !should_sync_discussion(&latest_updates, &discussion) {
            continue;
        }
        if !rebuild {
            info!(post_id = %discussion.post_id, "syncing discussion comments");
        }
        if let Err(err) =
            sync_discussion_with_client(state, &client, &discussion.discussion_id).await
        {
            stats.errors += 1;
            warn!(error = %err, discussion_id = %discussion.discussion_id, "discussion sync failed");
        } else {
            stats.discussions_synced += 1;
        }
    }

    {
        let mut health = state.admin_health.lock().await;
        health.comments_sync_last_success = Some(Utc::now());
    }
    Ok(stats)
}

pub fn is_enabled(config: &AppConfig) -> bool {
    CommentsConfig::from_app(config).is_some()
}

pub async fn sync_discussion_by_id(state: &AppState, discussion_id: &str) -> Result<(), JobError> {
    let Some(config) = CommentsConfig::from_app(&state.config) else {
        return Ok(());
    };
    let client = GithubAppClient::new(
        state.http_client.clone(),
        config.app_id,
        config.installation_id,
        config.private_key.clone(),
    );
    sync_discussion_with_client(state, &client, discussion_id).await
}

async fn sync_discussion_with_client(
    state: &AppState,
    client: &GithubAppClient,
    discussion_id: &str,
) -> Result<(), JobError> {
    let Some(pool) = state.db.as_ref() else {
        return Err(JobError::Comments("db not configured".to_string()));
    };
    let info = client.fetch_discussion_by_id(discussion_id).await?;
    let post_id = match find_discussion_by_discussion_id(pool, discussion_id).await? {
        Some(record) => record.post_id,
        None => post_id_from_title(state, &info.title).await?,
    };
    store_discussion(pool, &post_id, &info).await?;
    Ok(())
}

const PRECHECK_BATCH_SIZE: usize = 50;

async fn fetch_discussion_updates(
    client: &GithubAppClient,
    discussions: &[DiscussionRecord],
) -> Result<HashMap<String, DateTime<Utc>>, JobError> {
    let mut updates = HashMap::new();
    let ids: Vec<String> = discussions
        .iter()
        .filter(|discussion| is_github_discussion_id(&discussion.discussion_id))
        .map(|discussion| discussion.discussion_id.clone())
        .collect();
    for chunk in ids.chunks(PRECHECK_BATCH_SIZE) {
        let chunk_updates = client
            .fetch_discussion_updates(chunk)
            .await
            .map_err(|err| JobError::Comments(err.to_string()))?;
        updates.extend(chunk_updates);
    }
    Ok(updates)
}

fn is_github_discussion_id(value: &str) -> bool {
    value.starts_with("D_")
}

fn should_sync_discussion(
    latest_updates: &HashMap<String, DateTime<Utc>>,
    discussion: &DiscussionRecord,
) -> bool {
    match latest_updates.get(&discussion.discussion_id) {
        Some(updated_at) => *updated_at != discussion.updated_at,
        None => true,
    }
}

async fn post_id_from_title(state: &AppState, title: &str) -> Result<String, JobError> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Err(JobError::Comments("empty discussion title".to_string()));
    }
    if trimmed.starts_with('/') {
        return normalize_post_id(trimmed);
    }
    let (slug_raw, explicit_posts) = if let Some(value) = trimmed.strip_prefix("posts/") {
        (value, true)
    } else {
        (trimmed, false)
    };
    let slug = slug_raw.trim_matches('/');
    let slug = Slug::try_from(slug)
        .map_err(|err| JobError::Comments(err.to_string()))?
        .as_str()
        .to_string();
    if !explicit_posts {
        let valid_paths = state.valid_paths.read().await;
        let special = format!("/{slug}/");
        if valid_paths.contains(&special) {
            return Ok(special);
        }
    }
    Ok(format!("/posts/{slug}/"))
}

fn normalize_post_id(value: &str) -> Result<String, JobError> {
    let trimmed = value.trim();
    if trimmed.is_empty() || !trimmed.starts_with('/') {
        return Err(JobError::Comments("invalid post id".to_string()));
    }
    if trimmed.chars().any(|ch| ch.is_whitespace()) {
        return Err(JobError::Comments("invalid post id".to_string()));
    }
    Ok(trimmed.to_string())
}

struct EnsureOutcome {
    created: bool,
}

async fn ensure_discussion_for_post(
    client: &GithubAppClient,
    pool: &inkstone_infra::db::DbPool,
    config: &CommentsConfig,
    post: &PostRef,
) -> Result<EnsureOutcome, JobError> {
    let post_id = normalize_post_id(&post.post_id)?;
    if find_discussion_by_post_id(pool, &post_id).await?.is_some() {
        return Ok(EnsureOutcome { created: false });
    }

    for candidate in title_candidates(post) {
        if let Some(found) = client
            .find_discussion_by_title(&config.repo_owner, &config.repo_name, &candidate)
            .await?
        {
            store_discussion(pool, &post_id, &found).await?;
            return Ok(EnsureOutcome { created: false });
        }
    }

    let Some(category_id) = config.discussion_category_id.as_deref() else {
        warn!(
            post_id,
            "discussion category not configured; skipping create"
        );
        return Ok(EnsureOutcome { created: false });
    };

    let body = build_discussion_body(post);
    let created = client
        .create_discussion(
            &config.repo_owner,
            &config.repo_name,
            category_id,
            &post.post_id,
            &body,
        )
        .await?;
    store_discussion(pool, &post_id, &created).await?;
    Ok(EnsureOutcome { created: true })
}

async fn store_discussion(
    pool: &inkstone_infra::db::DbPool,
    post_id: &str,
    info: &DiscussionInfo,
) -> Result<(), JobError> {
    let discussion = DiscussionRecord {
        post_id: post_id.to_string(),
        discussion_id: info.id.clone(),
        number: info.number,
        title: info.title.clone(),
        url: info.url.clone(),
        created_at: info.created_at,
        updated_at: info.updated_at,
    };
    upsert_discussion(pool, &discussion).await?;
    let comments = flatten_comments(info);
    replace_comments(pool, &discussion.discussion_id, &comments).await?;
    Ok(())
}

fn flatten_comments(info: &DiscussionInfo) -> Vec<CommentRecord> {
    let mut records = Vec::new();
    for comment in &info.comments {
        records.push(CommentRecord {
            discussion_id: info.id.clone(),
            comment_id: comment.id.clone(),
            parent_id: None,
            comment_url: comment.url.clone(),
            source: "github".to_string(),
            author_login: comment.author_login.clone(),
            author_url: comment.author_url.clone(),
            author_avatar_url: comment.author_avatar_url.clone(),
            body_html: comment.body_html.clone(),
            created_at: comment.created_at,
            updated_at: comment.updated_at,
        });
        for reply in &comment.replies {
            records.push(CommentRecord {
                discussion_id: info.id.clone(),
                comment_id: reply.id.clone(),
                parent_id: Some(comment.id.clone()),
                comment_url: reply.url.clone(),
                source: "github".to_string(),
                author_login: reply.author_login.clone(),
                author_url: reply.author_url.clone(),
                author_avatar_url: reply.author_avatar_url.clone(),
                body_html: reply.body_html.clone(),
                created_at: reply.created_at,
                updated_at: reply.updated_at,
            });
        }
    }
    records
}

async fn fetch_posts(state: &AppState) -> Result<Vec<PostRef>, JobError> {
    let response = state
        .http_client
        .get(&state.config.feed_url)
        .send()
        .await?
        .error_for_status()?;
    let body = response.bytes().await?;
    let entries =
        parse_search_index_entries(&body).map_err(|err| JobError::Comments(err.to_string()))?;
    let mut posts = Vec::new();
    for entry in entries {
        if let Some(post) = post_from_index_entry(&entry) {
            posts.push(post);
        }
    }
    Ok(add_special_pages(posts))
}

fn post_from_index_entry(entry: &SearchIndexEntry) -> Option<PostRef> {
    let path = path_from_url(&entry.url)?;
    let slug = slug_from_path(&path)?;
    let post_id = format!("/posts/{slug}/");
    Some(PostRef {
        post_id,
        slug,
        title: entry.title.trim().to_string(),
        summary: summary_from_entry(entry),
    })
}

const SPECIAL_PAGES: [&str; 6] = [
    "/life/",
    "/life-en/",
    "/about/",
    "/about-en/",
    "/friends/",
    "/friends-en/",
];

fn add_special_pages(mut posts: Vec<PostRef>) -> Vec<PostRef> {
    let mut seen = std::collections::HashSet::new();
    for post in &posts {
        seen.insert(post.post_id.clone());
    }
    for path in SPECIAL_PAGES {
        if seen.insert(path.to_string()) {
            let slug = path.trim_matches('/').to_string();
            posts.push(PostRef {
                post_id: path.to_string(),
                slug,
                title: path.to_string(),
                summary: None,
            });
        }
    }
    posts
}

fn title_candidates(post: &PostRef) -> Vec<String> {
    let default_posts = format!("/posts/{}/", post.slug);
    let mut candidates = vec![
        post.slug.clone(),
        format!("posts/{}/", post.slug),
        default_posts.clone(),
    ];
    if post.post_id != default_posts {
        candidates.push(post.post_id.clone());
        if let Some(stripped) = post.post_id.strip_prefix('/') {
            candidates.push(stripped.to_string());
        }
    }
    let mut seen = std::collections::HashSet::new();
    candidates
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn path_from_url(url: &str) -> Option<String> {
    let path = if let Some(scheme_idx) = url.find("://") {
        let rest = &url[scheme_idx + 3..];
        let path_idx = rest.find('/')?;
        &rest[path_idx..]
    } else if url.starts_with('/') {
        url
    } else {
        return None;
    };
    let path = path.split(|ch| ch == '?' || ch == '#').next()?.trim();
    if path.is_empty() {
        None
    } else {
        Some(path.to_string())
    }
}

fn slug_from_path(path: &str) -> Option<String> {
    let marker = "/posts/";
    if !path.starts_with(marker) {
        return None;
    }
    let rest = &path[marker.len()..];
    let slug_raw = rest.split('/').next()?.trim();
    if slug_raw.is_empty() {
        return None;
    }
    let slug = slug_raw.to_ascii_lowercase();
    Slug::try_from(slug.as_str())
        .ok()
        .map(|value| value.as_str().to_string())
}

fn summary_from_entry(entry: &SearchIndexEntry) -> Option<String> {
    let (raw, has_more) = split_at_more_marker(&entry.content);
    let content = sanitize_markdown(raw);
    let cleaned = normalize_whitespace(&strip_html_tags(&content));
    if cleaned.is_empty() {
        return None;
    }
    if has_more {
        Some(cleaned)
    } else {
        Some(truncate_chars(&cleaned, 200))
    }
}

fn split_at_more_marker(content: &str) -> (&str, bool) {
    let markers = ["<!--more-->", "<!-- more -->"];
    let mut index: Option<usize> = None;
    for marker in markers {
        if let Some(pos) = content.find(marker) {
            index = Some(index.map_or(pos, |current| current.min(pos)));
        }
    }
    if let Some(pos) = index {
        (&content[..pos], true)
    } else {
        (content, false)
    }
}

const BLOG_BASE_URL: &str = "https://blog.itswincer.com";

fn build_discussion_body(post: &PostRef) -> String {
    let mut lines = Vec::new();
    lines.push(format!("## {}", post.title.trim()));
    if let Some(summary) = post.summary.as_ref() {
        let cleaned = normalize_whitespace(&strip_html_tags(summary));
        let snippet = truncate_chars(&cleaned, 200);
        if !snippet.is_empty() {
            lines.push(snippet);
        }
    }
    lines.push("\n---".to_string());
    lines.push(format!("{}{}", BLOG_BASE_URL, post.post_id));
    lines.join("\n")
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut output = String::new();
    for ch in value.chars().take(max_chars) {
        output.push(ch);
    }
    output
}

fn sanitize_markdown(value: &str) -> String {
    value
        .replace("<!--more-->", "")
        .replace("<!-- more -->", "")
}

fn strip_html_tags(input: &str) -> String {
    let mut text = input.to_string();
    for _ in 0..2 {
        let decoded = decode_html_entities(&text);
        text = remove_html_tags(&decoded);
    }
    decode_html_entities(&text)
}

fn normalize_whitespace(input: &str) -> String {
    let mut parts = input.split_whitespace();
    let Some(first) = parts.next() else {
        return String::new();
    };
    let mut output = String::with_capacity(input.len());
    output.push_str(first);
    for part in parts {
        output.push(' ');
        output.push_str(part);
    }
    output
}

fn remove_html_tags(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut in_tag = false;
    for ch in input.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                output.push(' ');
            }
            _ if !in_tag => output.push(ch),
            _ => {}
        }
    }
    output
}

fn decode_html_entities(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '&' {
            output.push(ch);
            continue;
        }

        let mut entity = String::new();
        let mut valid = false;
        while let Some(&next) = chars.peek() {
            if next == ';' {
                chars.next();
                valid = true;
                break;
            }
            if next.is_whitespace() || entity.len() > 32 {
                break;
            }
            chars.next();
            entity.push(next);
        }

        if !valid {
            output.push('&');
            output.push_str(&entity);
            continue;
        }

        match entity.as_str() {
            "lt" => output.push('<'),
            "gt" => output.push('>'),
            "amp" => output.push('&'),
            "quot" => output.push('"'),
            "apos" => output.push('\''),
            "nbsp" => output.push(' '),
            _ => {
                output.push('&');
                output.push_str(&entity);
                output.push(';');
            }
        }
    }
    output
}

impl CommentsConfig {
    fn from_app(config: &AppConfig) -> Option<Self> {
        Some(Self {
            app_id: config.github_app_id?,
            installation_id: config.github_app_installation_id?,
            private_key: config.github_app_private_key.clone()?,
            repo_owner: config.github_repo_owner.clone()?,
            repo_name: config.github_repo_name.clone()?,
            discussion_category_id: config.github_discussion_category_id.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::{TimeZone, Utc};

    use super::{
        SearchIndexEntry, is_github_discussion_id, path_from_url, should_sync_discussion,
        slug_from_path, title_candidates,
    };
    use inkstone_infra::db::DiscussionRecord;

    #[test]
    fn path_from_url_extracts_path() {
        let url = "https://example.com/posts/hello-world/?a=1";
        assert_eq!(path_from_url(url), Some("/posts/hello-world/".to_string()));
    }

    #[test]
    fn slug_from_path_extracts_posts() {
        let path = "/posts/hello-world/";
        assert_eq!(slug_from_path(path), Some("hello-world".to_string()));
    }

    #[test]
    fn slug_from_path_normalizes_case() {
        let path = "/posts/Hello-World/";
        assert_eq!(slug_from_path(path), Some("hello-world".to_string()));
    }

    #[test]
    fn slug_from_path_rejects_non_posts() {
        let path = "/about/";
        assert!(slug_from_path(path).is_none());
    }

    #[test]
    fn title_candidates_includes_legacy_posts() {
        let post = super::PostRef {
            post_id: "/posts/hello-world/".to_string(),
            slug: "hello-world".to_string(),
            title: "hello-world".to_string(),
            summary: None,
        };
        let candidates = title_candidates(&post);
        assert!(candidates.iter().any(|value| value == "posts/hello-world/"));
        assert!(
            candidates
                .iter()
                .any(|value| value == "/posts/hello-world/")
        );
    }

    #[test]
    fn add_special_pages_dedupes() {
        let posts = vec![super::PostRef {
            post_id: "/life/".to_string(),
            slug: "life".to_string(),
            title: "/life/".to_string(),
            summary: None,
        }];
        let updated = super::add_special_pages(posts);
        let life_count = updated
            .iter()
            .filter(|post| post.post_id == "/life/")
            .count();
        assert_eq!(life_count, 1);
        assert!(updated.iter().any(|post| post.post_id == "/about/"));
        assert!(updated.iter().any(|post| post.post_id == "/friends-en/"));
    }

    #[test]
    fn should_sync_discussion_compares_updated_at() {
        let discussion = DiscussionRecord {
            post_id: "/posts/hello/".to_string(),
            discussion_id: "D1".to_string(),
            number: 1,
            title: "/posts/hello/".to_string(),
            url: "https://github.com/example/1".to_string(),
            created_at: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap(),
        };
        let mut updates = HashMap::new();
        updates.insert(
            "D1".to_string(),
            Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap(),
        );
        assert!(!should_sync_discussion(&updates, &discussion));
        updates.insert(
            "D1".to_string(),
            Utc.with_ymd_and_hms(2024, 1, 3, 0, 0, 0).unwrap(),
        );
        assert!(should_sync_discussion(&updates, &discussion));
        updates.clear();
        assert!(should_sync_discussion(&updates, &discussion));
    }

    #[test]
    fn is_github_discussion_id_detects_prefix() {
        assert!(is_github_discussion_id("D_123"));
        assert!(!is_github_discussion_id("/posts/hello/"));
        assert!(!is_github_discussion_id("legacy:posts/hello/"));
    }

    #[test]
    fn build_discussion_body_uses_summary() {
        let post = super::PostRef {
            post_id: "/posts/hello-world/".to_string(),
            slug: "hello-world".to_string(),
            title: "Hello World".to_string(),
            summary: Some("Hello <b>World</b>".to_string()),
        };
        let body = super::build_discussion_body(&post);
        assert!(body.contains("## Hello World"));
        assert!(body.contains("Hello World"));
        assert!(body.contains("---"));
        assert!(body.contains("https://blog.itswincer.com/posts/hello-world/"));
    }

    #[test]
    fn summary_uses_more_marker() {
        let entry = SearchIndexEntry {
            title: "Title".to_string(),
            subtitle: None,
            url: "/posts/hello/".to_string(),
            date: "2025-01-01T00:00:00Z".to_string(),
            updated: None,
            category: None,
            tags: vec![],
            content: "Hello<!--more-->World".to_string(),
        };

        let summary = super::summary_from_entry(&entry).unwrap();
        assert_eq!(summary, "Hello");
    }

    #[test]
    fn summary_truncates_without_more() {
        let entry = SearchIndexEntry {
            title: "Title".to_string(),
            subtitle: None,
            url: "/posts/hello/".to_string(),
            date: "2025-01-01T00:00:00Z".to_string(),
            updated: None,
            category: None,
            tags: vec![],
            content: "a".repeat(250),
        };

        let summary = super::summary_from_entry(&entry).unwrap();
        assert_eq!(summary.chars().count(), 200);
    }
}
