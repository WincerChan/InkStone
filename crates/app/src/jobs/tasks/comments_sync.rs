use std::io::Cursor;

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use tracing::{info, warn};

use crate::config::AppConfig;
use crate::jobs::JobError;
use crate::state::AppState;
use inkstone_core::types::slug::Slug;
use inkstone_infra::db::{
    find_discussion_by_discussion_id, find_discussion_by_post_id, list_discussions,
    replace_comments, upsert_discussion, CommentRecord, DiscussionRecord,
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
    url: String,
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
    // Temporary: limit sync to the first two posts for manual verification.
    let posts = apply_post_limit(posts, 2);
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
    for discussion in discussions {
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
        warn!(post_id, "discussion category not configured; skipping create");
        return Ok(EnsureOutcome { created: false });
    };

    let body = format!(
        "Auto-generated discussion for {url}\n\nPath: `{path}`\n\nGenerated by Inkstone.",
        url = post.url,
        path = post.post_id
    );
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
            author_login: comment.author_login.clone(),
            author_url: comment.author_url.clone(),
            body_html: comment.body_html.clone(),
            created_at: comment.created_at,
            updated_at: comment.updated_at,
        });
        for reply in &comment.replies {
            records.push(CommentRecord {
                discussion_id: info.id.clone(),
                comment_id: reply.id.clone(),
                parent_id: Some(comment.id.clone()),
                author_login: reply.author_login.clone(),
                author_url: reply.author_url.clone(),
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
    parse_feed_posts(&body).map_err(|err| JobError::Comments(err.to_string()))
}

fn parse_feed_posts(xml: &[u8]) -> Result<Vec<PostRef>, quick_xml::Error> {
    let mut reader = Reader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut entries = Vec::new();
    let mut current: Option<String> = None;

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(event) => {
                if event.name().as_ref() == b"entry" {
                    current = Some(String::new());
                } else if event.name().as_ref() == b"link" {
                    if let Some(link) = current.as_mut() {
                        parse_link_attrs(&event, link)?;
                    }
                }
            }
            Event::Empty(event) => {
                if event.name().as_ref() == b"link" {
                    if let Some(link) = current.as_mut() {
                        parse_link_attrs(&event, link)?;
                    }
                }
            }
            Event::End(event) => {
                if event.name().as_ref() == b"entry" {
                    if let Some(link) = current.take() {
                        if let Some(post) = post_from_link(&link) {
                            entries.push(post);
                        }
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    Ok(entries)
}

fn parse_link_attrs(event: &BytesStart<'_>, link: &mut String) -> Result<(), quick_xml::Error> {
    let mut href = None;
    let mut rel = None;
    for attr in event.attributes() {
        let attr = attr?;
        match attr.key.as_ref() {
            b"href" => href = Some(attr.unescape_value()?.to_string()),
            b"rel" => rel = Some(attr.unescape_value()?.to_string()),
            _ => {}
        }
    }
    let rel_alt = rel.as_deref() == Some("alternate");
    if let Some(href) = href {
        if link.is_empty() || rel_alt {
            link.clear();
            link.push_str(&href);
        }
    }
    Ok(())
}

fn post_from_link(link: &str) -> Option<PostRef> {
    let path = path_from_url(link)?;
    let slug = slug_from_path(&path)?;
    Some(PostRef {
        post_id: path,
        slug,
        url: link.to_string(),
    })
}

fn title_candidates(post: &PostRef) -> Vec<String> {
    let default_posts = format!("/posts/{}/", post.slug);
    let mut candidates = vec![
        post.slug.clone(),
        format!("posts/{}", post.slug),
        default_posts.clone(),
        format!("/posts/{}", post.slug),
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
    let path = path
        .split(|ch| ch == '?' || ch == '#')
        .next()?
        .trim();
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
    let slug = rest.split('/').next()?.trim();
    if slug.is_empty() {
        return None;
    }
    Slug::try_from(slug).ok().map(|value| value.as_str().to_string())
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

fn apply_post_limit(mut posts: Vec<PostRef>, limit: usize) -> Vec<PostRef> {
    if limit == 0 || posts.len() <= limit {
        return posts;
    }
    posts.truncate(limit);
    posts
}

#[cfg(test)]
mod tests {
    use super::{path_from_url, slug_from_path, title_candidates};

    #[test]
    fn path_from_url_extracts_path() {
        let url = "https://example.com/posts/hello-world/?a=1";
        assert_eq!(
            path_from_url(url),
            Some("/posts/hello-world/".to_string())
        );
    }

    #[test]
    fn slug_from_path_extracts_posts() {
        let path = "/posts/hello-world/";
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
            url: "https://example.com/posts/hello-world/".to_string(),
        };
        let candidates = title_candidates(&post);
        assert!(candidates.iter().any(|value| value == "posts/hello-world"));
        assert!(candidates.iter().any(|value| value == "/posts/hello-world/"));
    }

    #[test]
    fn apply_post_limit_truncates() {
        let posts = vec![
            super::PostRef {
                post_id: "/posts/a/".to_string(),
                slug: "a".to_string(),
                url: "https://example.com/posts/a/".to_string(),
            },
            super::PostRef {
                post_id: "/posts/b/".to_string(),
                slug: "b".to_string(),
                url: "https://example.com/posts/b/".to_string(),
            },
            super::PostRef {
                post_id: "/posts/c/".to_string(),
                slug: "c".to_string(),
                url: "https://example.com/posts/c/".to_string(),
            },
        ];
        let limited = super::apply_post_limit(posts, 2);
        assert_eq!(limited.len(), 2);
    }
}
