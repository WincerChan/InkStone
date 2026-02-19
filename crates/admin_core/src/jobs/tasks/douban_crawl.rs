use std::collections::HashSet;

use aws_credential_types::Credentials;
use aws_credential_types::provider::SharedCredentialsProvider;
use aws_sdk_s3::Client as S3Client;
use aws_sdk_s3::config::BehaviorVersion;
use aws_sdk_s3::config::Builder as S3ConfigBuilder;
use aws_sdk_s3::primitives::ByteStream;
use aws_types::region::Region;
use chrono::{NaiveDate, Utc};
use scraper::{ElementRef, Html, Selector};
use serde::Serialize;
use tracing::{debug, warn};

use crate::jobs::JobError;
use crate::state::AdminState;
use inkstone_infra::db::{
    DbPool, DoubanItemRecord, fetch_existing_douban_item_ids, insert_douban_items,
    upsert_douban_items,
};

const ITEM_LOG_LIMIT: usize = 20;
const DOUBAN_REFERER: &str = "https://www.douban.com";
const MAX_POSTER_BYTES: usize = 10 * 1024 * 1024;
// CI note: this task is in admin_core and should only trigger admin image builds.

#[derive(Clone)]
struct PosterR2Uploader {
    client: S3Client,
    bucket: String,
    public_base_url: String,
}

struct DownloadedPoster {
    bytes: Vec<u8>,
    content_type: String,
    extension: String,
}

impl PosterR2Uploader {
    fn from_state(state: &AdminState) -> Option<Self> {
        let endpoint = state.config.douban_poster_r2_endpoint.clone();
        let bucket = state.config.douban_poster_r2_bucket.clone();
        let access_key_id = state.config.douban_poster_r2_access_key_id.clone();
        let secret_access_key = state.config.douban_poster_r2_secret_access_key.clone();
        let public_base_url = state.config.douban_poster_r2_public_base_url.clone();

        let any_configured = [
            endpoint.as_deref(),
            bucket.as_deref(),
            access_key_id.as_deref(),
            secret_access_key.as_deref(),
            public_base_url.as_deref(),
        ]
        .iter()
        .any(|value| value.is_some());
        if !any_configured {
            return None;
        }

        let (
            Some(endpoint),
            Some(bucket),
            Some(access_key_id),
            Some(secret_access_key),
            Some(public_base_url),
        ) = (
            endpoint,
            bucket,
            access_key_id,
            secret_access_key,
            public_base_url,
        )
        else {
            warn!("douban poster r2 config is partial; poster upload disabled");
            return None;
        };

        let credentials = Credentials::new(
            access_key_id,
            secret_access_key,
            None,
            None,
            "inkstone-douban-crawl",
        );
        let config = S3ConfigBuilder::new()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new(state.config.douban_poster_r2_region.clone()))
            .credentials_provider(SharedCredentialsProvider::new(credentials))
            .endpoint_url(endpoint)
            .force_path_style(true)
            .build();

        Some(Self {
            client: S3Client::from_conf(config),
            bucket,
            public_base_url: public_base_url.trim_end_matches('/').to_string(),
        })
    }

    fn object_key(&self, category: DoubanCategory, item_id: &str, extension: &str) -> String {
        build_poster_object_key(category, item_id, extension)
    }

    fn public_url(&self, key: &str) -> String {
        format!("{}/{}", self.public_base_url, key)
    }

    fn is_managed_url(&self, url: &str) -> bool {
        url.starts_with(&self.public_base_url)
    }
}

#[derive(Debug, Serialize)]
pub struct DoubanItem {
    pub id: String,
    pub tags: Vec<String>,
    pub date: Option<String>,
    pub comment: Option<String>,
    pub rating: Option<u8>,
    pub title: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub poster: Option<String>,
}

#[derive(Clone, Copy)]
pub enum DoubanCategory {
    Movie,
    Book,
    Game,
}

impl DoubanCategory {
    fn label(self) -> &'static str {
        match self {
            DoubanCategory::Movie => "movie",
            DoubanCategory::Book => "book",
            DoubanCategory::Game => "game",
        }
    }

    fn base_url(self) -> &'static str {
        match self {
            DoubanCategory::Movie => "https://movie.douban.com",
            DoubanCategory::Book => "https://book.douban.com",
            DoubanCategory::Game => "https://www.douban.com",
        }
    }

    fn start_url(self, uid: &str) -> String {
        match self {
            DoubanCategory::Movie => format!("{}/people/{}/collect", self.base_url(), uid),
            DoubanCategory::Book => format!("{}/people/{}/collect", self.base_url(), uid),
            DoubanCategory::Game => {
                format!("{}/people/{}/games?action=collect", self.base_url(), uid)
            }
        }
    }
}

pub async fn run(state: &AdminState, rebuild: bool) -> Result<(), JobError> {
    {
        let mut health = state.admin_health.lock().await;
        health.douban_crawl_last_run = Some(Utc::now());
    }
    let uid = state.config.douban_uid.as_str();
    for category in [
        DoubanCategory::Movie,
        DoubanCategory::Book,
        DoubanCategory::Game,
    ] {
        let items = fetch_all_pages(state, category, uid, rebuild).await?;
        log_items(category, &items);
    }
    {
        let mut health = state.admin_health.lock().await;
        health.douban_crawl_last_success = Some(Utc::now());
    }
    Ok(())
}

pub async fn run_for_category(
    state: &AdminState,
    rebuild: bool,
    category: DoubanCategory,
) -> Result<(), JobError> {
    {
        let mut health = state.admin_health.lock().await;
        health.douban_crawl_last_run = Some(Utc::now());
    }
    let uid = state.config.douban_uid.as_str();
    let items = fetch_all_pages(state, category, uid, rebuild).await?;
    log_items(category, &items);
    {
        let mut health = state.admin_health.lock().await;
        health.douban_crawl_last_success = Some(Utc::now());
    }
    Ok(())
}

async fn fetch_all_pages(
    state: &AdminState,
    category: DoubanCategory,
    uid: &str,
    rebuild: bool,
) -> Result<Vec<DoubanItem>, JobError> {
    let poster_uploader = PosterR2Uploader::from_state(state);
    let mut items = Vec::new();
    let mut next_url = category.start_url(uid);
    let mut seen = HashSet::new();
    let mut pages_fetched = 0;
    let max_pages = state.config.douban_max_pages;
    let pool = state.db.as_ref();
    if pool.is_none() {
        warn!(
            category = category.label(),
            "db not configured; skip douban upsert"
        );
    }

    loop {
        if !seen.insert(next_url.clone()) {
            break;
        }
        pages_fetched += 1;
        let html = fetch_page(state, &next_url).await?;
        let mut page_items = parse_page(&html, category);
        if let Some(uploader) = poster_uploader.as_ref() {
            match (rebuild, pool) {
                (true, _) | (_, None) => {
                    upload_page_posters(state, uploader, category, &mut page_items, None).await;
                }
                (false, Some(pool)) => {
                    upload_new_page_posters(state, uploader, pool, category, &mut page_items).await;
                }
            }
        }
        let stop_on_existing = if let Some(pool) = pool {
            sync_page(pool, category, &page_items, rebuild).await?
        } else {
            false
        };
        items.extend(page_items);

        if max_pages > 0 && pages_fetched >= max_pages {
            break;
        }
        if stop_on_existing && !rebuild {
            break;
        }
        let next_link = extract_next_link(&html, category.base_url());
        match next_link {
            Some(url) => next_url = url,
            None => break,
        }
    }

    Ok(items)
}

async fn fetch_page(state: &AdminState, url: &str) -> Result<String, JobError> {
    let mut request = state
        .http_client
        .get(url)
        .header("User-Agent", state.config.douban_user_agent.as_str());
    if !state.config.douban_cookie.trim().is_empty() {
        request = request.header("Cookie", state.config.douban_cookie.as_str());
    }
    let response = request.send().await?.error_for_status()?;
    Ok(response.text().await?)
}

async fn upload_page_posters(
    state: &AdminState,
    uploader: &PosterR2Uploader,
    category: DoubanCategory,
    items: &mut [DoubanItem],
    include_item_ids: Option<&HashSet<String>>,
) {
    for item in items {
        if include_item_ids
            .as_ref()
            .is_some_and(|ids| !ids.contains(&item.id))
        {
            continue;
        }
        let Some(source_url) = item.poster.as_deref() else {
            continue;
        };
        if uploader.is_managed_url(source_url) {
            continue;
        }
        match fetch_and_upload_poster(state, uploader, category, &item.id, source_url).await {
            Ok(public_url) => {
                item.poster = Some(public_url);
            }
            Err(err) => {
                warn!(
                    category = category.label(),
                    item_id = %item.id,
                    source_url,
                    error = %err,
                    "douban poster upload failed; keep source url"
                );
            }
        }
    }
}

async fn upload_new_page_posters(
    state: &AdminState,
    uploader: &PosterR2Uploader,
    pool: &DbPool,
    category: DoubanCategory,
    items: &mut [DoubanItem],
) {
    let item_ids: Vec<String> = items.iter().map(|item| item.id.clone()).collect();
    let existing_ids = match fetch_existing_douban_item_ids(pool, category.label(), &item_ids).await
    {
        Ok(ids) => ids,
        Err(err) => {
            warn!(
                category = category.label(),
                error = %err,
                "load existing douban item ids failed; fallback to uploading all posters"
            );
            HashSet::new()
        }
    };
    let upload_item_ids = collect_refresh_upload_item_ids(items, &existing_ids);
    if upload_item_ids.is_empty() {
        return;
    }
    upload_page_posters(state, uploader, category, items, Some(&upload_item_ids)).await;
}

fn collect_refresh_upload_item_ids(
    items: &[DoubanItem],
    existing_ids: &HashSet<String>,
) -> HashSet<String> {
    items
        .iter()
        .filter_map(|item| {
            if existing_ids.contains(&item.id) {
                None
            } else {
                Some(item.id.clone())
            }
        })
        .collect()
}

async fn fetch_and_upload_poster(
    state: &AdminState,
    uploader: &PosterR2Uploader,
    category: DoubanCategory,
    item_id: &str,
    source_url: &str,
) -> Result<String, String> {
    let poster = download_poster(state, source_url).await?;
    let key = uploader.object_key(category, item_id, &poster.extension);
    uploader
        .client
        .put_object()
        .bucket(&uploader.bucket)
        .key(&key)
        .content_type(poster.content_type)
        .body(ByteStream::from(poster.bytes))
        .send()
        .await
        .map_err(|err| format!("upload object failed: {err}"))?;
    Ok(uploader.public_url(&key))
}

async fn download_poster(state: &AdminState, url: &str) -> Result<DownloadedPoster, String> {
    let response = state
        .http_client
        .get(url)
        .header("User-Agent", state.config.douban_user_agent.as_str())
        .header("Referer", DOUBAN_REFERER)
        .send()
        .await
        .map_err(|err| format!("download request failed: {err}"))?
        .error_for_status()
        .map_err(|err| format!("download request status error: {err}"))?;
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok());
    let (content_type, extension) = resolve_poster_media_type(content_type, url);
    let bytes = response
        .bytes()
        .await
        .map_err(|err| format!("read response bytes failed: {err}"))?;
    if bytes.len() > MAX_POSTER_BYTES {
        return Err(format!(
            "poster too large: {} bytes (limit {})",
            bytes.len(),
            MAX_POSTER_BYTES
        ));
    }
    Ok(DownloadedPoster {
        bytes: bytes.to_vec(),
        content_type,
        extension,
    })
}

fn resolve_poster_media_type(content_type: Option<&str>, url: &str) -> (String, String) {
    if let Some(media) = parse_supported_image_content_type(content_type) {
        return media;
    }
    if let Some(media) = parse_supported_image_ext_from_url(url) {
        return media;
    }
    ("image/jpeg".to_string(), "jpg".to_string())
}

fn parse_supported_image_content_type(content_type: Option<&str>) -> Option<(String, String)> {
    let raw = content_type?
        .split(';')
        .next()
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let ext = match raw.as_str() {
        "image/jpeg" | "image/jpg" | "image/pjpeg" => "jpg",
        "image/png" => "png",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => return None,
    };
    let canonical = if ext == "jpg" {
        "image/jpeg".to_string()
    } else {
        raw
    };
    Some((canonical, ext.to_string()))
}

fn parse_supported_image_ext_from_url(url: &str) -> Option<(String, String)> {
    let without_query = url.split_once('?').map(|(path, _)| path).unwrap_or(url);
    let url_without_query = without_query
        .split_once('#')
        .map(|(path, _)| path)
        .unwrap_or(without_query);
    let ext = url_without_query
        .rsplit_once('.')
        .map(|(_, ext)| ext)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let (content_type, ext) = match ext.as_str() {
        "jpeg" | "jpg" => ("image/jpeg", "jpg"),
        "png" => ("image/png", "png"),
        "webp" => ("image/webp", "webp"),
        "gif" => ("image/gif", "gif"),
        _ => return None,
    };
    Some((content_type.to_string(), ext.to_string()))
}

fn parse_page(html: &str, category: DoubanCategory) -> Vec<DoubanItem> {
    match category {
        DoubanCategory::Movie => parse_movie_items(html),
        DoubanCategory::Book => parse_book_items(html),
        DoubanCategory::Game => parse_game_items(html),
    }
}

fn parse_movie_items(html: &str) -> Vec<DoubanItem> {
    let document = Html::parse_document(html);
    let item_selector = Selector::parse("div.item.comment-item").expect("selector");
    let title_selector = Selector::parse("li.title em").expect("selector");
    let title_fallback_selector = Selector::parse("li.title a").expect("selector");
    let poster_selector = Selector::parse("div.pic img").expect("selector");
    let rating_selector = Selector::parse("span[class^=\"rating\"]").expect("selector");
    let date_selector = Selector::parse("span.date").expect("selector");
    let comment_selector = Selector::parse("span.comment").expect("selector");
    let tags_selector = Selector::parse("span.tags").expect("selector");
    let link_selector = Selector::parse("div.pic a").expect("selector");

    document
        .select(&item_selector)
        .filter_map(|item| {
            let id = item
                .select(&link_selector)
                .next()
                .and_then(|link| link.value().attr("href"))
                .and_then(extract_id_from_url)?;
            let title = item
                .select(&title_selector)
                .next()
                .map(extract_text)
                .filter(|text| !text.is_empty())
                .or_else(|| {
                    item.select(&title_fallback_selector)
                        .next()
                        .map(extract_text)
                })?;
            let poster = item
                .select(&poster_selector)
                .next()
                .and_then(|img| img.value().attr("src"))
                .map(|value| value.to_string());
            let rating = item
                .select(&rating_selector)
                .next()
                .and_then(|node| node.value().attr("class"))
                .and_then(parse_rating_from_classes);
            let date = item
                .select(&date_selector)
                .next()
                .and_then(|node| extract_date(&extract_text(node)));
            let comment = item
                .select(&comment_selector)
                .next()
                .map(extract_text)
                .filter(|text| !text.is_empty());
            let tags = item
                .select(&tags_selector)
                .next()
                .map(|node| parse_tags_text(&extract_text(node)))
                .unwrap_or_default();

            Some(DoubanItem {
                id,
                tags,
                date,
                comment,
                rating,
                title,
                type_: DoubanCategory::Movie.label().to_string(),
                poster,
            })
        })
        .collect()
}

fn parse_book_items(html: &str) -> Vec<DoubanItem> {
    let document = Html::parse_document(html);
    let item_selector = Selector::parse("li.subject-item").expect("selector");
    let title_selector = Selector::parse("h2 a").expect("selector");
    let poster_selector = Selector::parse("div.pic img").expect("selector");
    let rating_selector = Selector::parse("span[class^=\"rating\"]").expect("selector");
    let date_selector = Selector::parse("span.date").expect("selector");
    let comment_selector = Selector::parse("p.comment").expect("selector");
    let tags_selector = Selector::parse("span.tags").expect("selector");
    let link_selector = Selector::parse("div.pic a").expect("selector");

    document
        .select(&item_selector)
        .filter_map(|item| {
            let id = item
                .select(&link_selector)
                .next()
                .and_then(|link| link.value().attr("href"))
                .and_then(extract_id_from_url)?;
            let title = item.select(&title_selector).next().map(extract_text)?;
            let poster = item
                .select(&poster_selector)
                .next()
                .and_then(|img| img.value().attr("src"))
                .map(|value| value.to_string());
            let rating = item
                .select(&rating_selector)
                .next()
                .and_then(|node| node.value().attr("class"))
                .and_then(parse_rating_from_classes);
            let date = item
                .select(&date_selector)
                .next()
                .and_then(|node| extract_date(&extract_text(node)));
            let comment = item
                .select(&comment_selector)
                .next()
                .map(extract_text)
                .filter(|text| !text.is_empty());
            let tags = item
                .select(&tags_selector)
                .next()
                .map(|node| parse_tags_text(&extract_text(node)))
                .unwrap_or_default();

            Some(DoubanItem {
                id,
                tags,
                date,
                comment,
                rating,
                title,
                type_: DoubanCategory::Book.label().to_string(),
                poster,
            })
        })
        .collect()
}

fn parse_game_items(html: &str) -> Vec<DoubanItem> {
    let document = Html::parse_document(html);
    let item_selector = Selector::parse("div.common-item").expect("selector");
    let title_selector = Selector::parse("div.title a").expect("selector");
    let poster_selector = Selector::parse("div.pic img").expect("selector");
    let rating_selector = Selector::parse("div.rating-info span.rating-star").expect("selector");
    let date_selector = Selector::parse("div.rating-info span.date").expect("selector");
    let tags_selector = Selector::parse("div.rating-info span.tags").expect("selector");
    let link_selector = Selector::parse("div.pic a").expect("selector");
    let content_selector = Selector::parse("div.content").expect("selector");

    document
        .select(&item_selector)
        .filter_map(|item| {
            let id = item
                .select(&link_selector)
                .next()
                .and_then(|link| link.value().attr("href"))
                .and_then(extract_id_from_url)?;
            let title = item.select(&title_selector).next().map(extract_text)?;
            let poster = item
                .select(&poster_selector)
                .next()
                .and_then(|img| img.value().attr("src"))
                .map(|value| value.to_string());
            let rating = item
                .select(&rating_selector)
                .next()
                .and_then(|node| node.value().attr("class"))
                .and_then(parse_rating_from_classes);
            let date = item
                .select(&date_selector)
                .next()
                .and_then(|node| extract_date(&extract_text(node)));
            let tags = item
                .select(&tags_selector)
                .next()
                .map(|node| parse_tags_text(&extract_text(node)))
                .unwrap_or_default();
            let comment = item
                .select(&content_selector)
                .next()
                .and_then(extract_game_comment);

            Some(DoubanItem {
                id,
                tags,
                date,
                comment,
                rating,
                title,
                type_: DoubanCategory::Game.label().to_string(),
                poster,
            })
        })
        .collect()
}

fn extract_game_comment(content: ElementRef<'_>) -> Option<String> {
    for child in content.children() {
        let Some(element) = ElementRef::wrap(child) else {
            continue;
        };
        if element.value().name() != "div" {
            continue;
        }
        let class = element.value().attr("class").unwrap_or_default();
        if class == "title" || class == "desc" || class == "user-operation" {
            continue;
        }
        let text = extract_text(element);
        if !text.is_empty() {
            return Some(text);
        }
    }
    None
}

fn extract_text(element: ElementRef<'_>) -> String {
    normalize_whitespace(&element.text().collect::<Vec<_>>().join(""))
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

fn extract_id_from_url(url: &str) -> Option<String> {
    for marker in ["subject/", "game/"] {
        if let Some(pos) = url.find(marker) {
            let start = pos + marker.len();
            let rest = &url[start..];
            let digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
            if !digits.is_empty() {
                return Some(digits);
            }
        }
    }
    None
}

fn parse_rating_from_classes(class_value: &str) -> Option<u8> {
    for class_name in class_value.split_whitespace() {
        if let Some(rating) = parse_rating_class(class_name) {
            return Some(rating);
        }
    }
    None
}

fn parse_rating_class(class_name: &str) -> Option<u8> {
    if let Some(rest) = class_name.strip_prefix("rating") {
        let digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
        return digits.parse::<u8>().ok();
    }
    if let Some(rest) = class_name.strip_prefix("allstar") {
        let digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
        if let Ok(value) = digits.parse::<u8>() {
            if value >= 10 {
                return Some(value / 10);
            }
        }
    }
    None
}

fn extract_date(text: &str) -> Option<String> {
    text.split_whitespace()
        .next()
        .map(|value| value.to_string())
}

fn parse_tags_text(text: &str) -> Vec<String> {
    let trimmed = text.trim();
    let stripped = trimmed.strip_prefix("标签:").unwrap_or(trimmed).trim();
    if stripped.is_empty() {
        return Vec::new();
    }
    stripped
        .split(|ch: char| ch == ',' || ch.is_whitespace())
        .filter(|part| !part.is_empty())
        .map(|part| part.to_string())
        .collect()
}

async fn sync_page(
    pool: &DbPool,
    category: DoubanCategory,
    items: &[DoubanItem],
    rebuild: bool,
) -> Result<bool, JobError> {
    let records = map_items_for_db(items);
    if records.is_empty() {
        return Ok(false);
    }

    if rebuild {
        let affected = upsert_douban_items(pool, &records).await?;
        debug!(
            category = category.label(),
            affected, "douban items upserted"
        );
        return Ok(false);
    }

    let total = records.len();
    let inserted = insert_douban_items(pool, &records).await?;
    debug!(
        category = category.label(),
        inserted, "douban items inserted"
    );
    let stop = should_stop_on_existing(inserted, total);
    if stop {
        debug!(
            category = category.label(),
            "douban pagination stopped after existing item"
        );
    }
    Ok(stop)
}

fn map_items_for_db(items: &[DoubanItem]) -> Vec<DoubanItemRecord> {
    items
        .iter()
        .map(|item| {
            let date = parse_date_for_record(item);
            DoubanItemRecord {
                id: item.id.clone(),
                item_type: item.type_.clone(),
                title: item.title.clone(),
                poster: item.poster.clone(),
                rating: item.rating.map(|value| value as i16),
                tags: item.tags.clone(),
                comment: item.comment.clone(),
                date,
            }
        })
        .collect()
}

fn parse_date_for_record(item: &DoubanItem) -> Option<NaiveDate> {
    item.date
        .as_deref()
        .and_then(|value| parse_date(value, &item.id))
}

fn should_stop_on_existing(inserted: u64, total: usize) -> bool {
    inserted < total as u64
}

fn parse_date(value: &str, item_id: &str) -> Option<NaiveDate> {
    match NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        Ok(date) => Some(date),
        Err(err) => {
            warn!(item_id, date = value, error = %err, "douban date parse failed");
            None
        }
    }
}

fn extract_next_link(html: &str, base_url: &str) -> Option<String> {
    let document = Html::parse_document(html);
    let selector = Selector::parse("link[rel=\"next\"]").expect("selector");
    let href = document
        .select(&selector)
        .next()
        .and_then(|node| node.value().attr("href"))?;
    Some(join_url(base_url, href))
}

fn join_url(base_url: &str, href: &str) -> String {
    if href.starts_with("http://") || href.starts_with("https://") {
        href.to_string()
    } else if href.starts_with('/') {
        format!("{base_url}{href}")
    } else {
        format!("{base_url}/{href}")
    }
}

fn build_poster_object_key(category: DoubanCategory, item_id: &str, extension: &str) -> String {
    let category_segment = sanitize_key_segment(category.label(), "item");
    let item_segment = sanitize_key_segment(item_id, "unknown");
    let extension = sanitize_key_segment(extension, "jpg").to_ascii_lowercase();
    format!("{category_segment}/{item_segment}.{extension}")
}

fn sanitize_key_segment(raw: &str, fallback: &str) -> String {
    let sanitized: String = raw
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    let sanitized = sanitized.trim_matches('_');
    if sanitized.is_empty() {
        fallback.to_string()
    } else {
        sanitized.to_string()
    }
}

fn log_items(category: DoubanCategory, items: &[DoubanItem]) {
    let label = category.label();
    debug!(category = label, total = items.len(), "douban crawl parsed");
    for item in items.iter().take(ITEM_LOG_LIMIT) {
        match serde_json::to_string(item) {
            Ok(json) => debug!(category = label, item = %json, "douban item"),
            Err(err) => warn!(category = label, error = %err, "douban item serialize failed"),
        }
    }
    if items.len() > ITEM_LOG_LIMIT {
        debug!(
            category = label,
            omitted = items.len().saturating_sub(ITEM_LOG_LIMIT),
            "douban items omitted from log"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DoubanCategory, DoubanItem, build_poster_object_key, collect_refresh_upload_item_ids,
        map_items_for_db, parse_book_items, parse_game_items, parse_movie_items,
        parse_supported_image_content_type, parse_supported_image_ext_from_url,
        sanitize_key_segment, should_stop_on_existing,
    };
    use std::collections::HashSet;

    #[test]
    fn parse_movie_item() {
        let html = r#"
        <div class="item comment-item">
            <div class="pic">
                <a href="https://movie.douban.com/subject/1234567/"><img src="poster.jpg"></a>
            </div>
            <div class="info">
                <ul>
                    <li class="title"><a><em>Movie Title</em></a></li>
                    <li><span class="rating3-t"></span><span class="date">2025-01-01</span></li>
                    <li><span class="comment">Great movie.</span><span class="tags">标签: tag1 tag2</span></li>
                </ul>
            </div>
        </div>
        "#;
        let items = parse_movie_items(html);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "1234567");
        assert_eq!(items[0].rating, Some(3));
        assert_eq!(items[0].date.as_deref(), Some("2025-01-01"));
        assert_eq!(items[0].tags, vec!["tag1", "tag2"]);
        assert_eq!(items[0].title, "Movie Title");
    }

    #[test]
    fn parse_book_item() {
        let html = r#"
        <li class="subject-item">
            <div class="pic">
                <a href="https://book.douban.com/subject/7654321/"><img src="cover.jpg"></a>
            </div>
            <div class="info">
                <h2><a>Book Title</a></h2>
                <div class="short-note">
                    <div><span class="rating4-t"></span><span class="date">2025-02-02 读过</span></div>
                    <p class="comment">Nice book.</p>
                </div>
            </div>
        </li>
        "#;
        let items = parse_book_items(html);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "7654321");
        assert_eq!(items[0].rating, Some(4));
        assert_eq!(items[0].date.as_deref(), Some("2025-02-02"));
        assert_eq!(items[0].comment.as_deref(), Some("Nice book."));
    }

    #[test]
    fn parse_game_item() {
        let html = r#"
        <div class="common-item">
            <div class="pic">
                <a href="https://www.douban.com/game/112233/"><img src="game.jpg"></a>
            </div>
            <div class="content">
                <div class="title"><a>Game Title</a></div>
                <div class="desc">
                    <div class="rating-info">
                        <span class="rating-star allstar40"></span>
                        <span class="date">2024-12-12</span>
                        <span class="tags">标签: tagA tagB</span>
                    </div>
                </div>
                <div>Fun game.</div>
                <div class="user-operation"></div>
            </div>
        </div>
        "#;
        let items = parse_game_items(html);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "112233");
        assert_eq!(items[0].rating, Some(4));
        assert_eq!(items[0].date.as_deref(), Some("2024-12-12"));
        assert_eq!(items[0].comment.as_deref(), Some("Fun game."));
        assert_eq!(items[0].tags, vec!["tagA", "tagB"]);
    }

    #[test]
    fn map_items_parses_dates() {
        let items = vec![DoubanItem {
            id: "1".to_string(),
            tags: vec![],
            date: Some("2025-12-20".to_string()),
            comment: None,
            rating: Some(4),
            title: "Title".to_string(),
            type_: "movie".to_string(),
            poster: None,
        }];
        let records = map_items_for_db(&items);
        assert_eq!(records.len(), 1);
        assert!(records[0].date.is_some());
    }

    #[test]
    fn should_stop_on_existing_conflict() {
        assert!(should_stop_on_existing(0, 1));
        assert!(should_stop_on_existing(2, 3));
        assert!(!should_stop_on_existing(3, 3));
    }

    #[test]
    fn build_poster_object_key_uses_extension() {
        let key = build_poster_object_key(DoubanCategory::Movie, "123456", "jpg");
        assert_eq!(key, "movie/123456.jpg");
    }

    #[test]
    fn parse_supported_image_content_type_maps_jpeg() {
        let media = parse_supported_image_content_type(Some("image/jpeg; charset=binary"));
        assert_eq!(media, Some(("image/jpeg".to_string(), "jpg".to_string())));
    }

    #[test]
    fn parse_supported_image_ext_from_url_maps_webp() {
        let media = parse_supported_image_ext_from_url("https://img.example.com/a/b/c.webp?x=1");
        assert_eq!(media, Some(("image/webp".to_string(), "webp".to_string())));
    }

    #[test]
    fn sanitize_key_segment_applies_fallback() {
        assert_eq!(sanitize_key_segment("中文 空格", "fallback"), "fallback");
        assert_eq!(sanitize_key_segment("abc-123", "fallback"), "abc-123");
    }

    #[test]
    fn collect_refresh_upload_item_ids_excludes_existing_ids() {
        let items = vec![
            DoubanItem {
                id: "1".to_string(),
                tags: vec![],
                date: None,
                comment: None,
                rating: None,
                title: "Item 1".to_string(),
                type_: "movie".to_string(),
                poster: None,
            },
            DoubanItem {
                id: "2".to_string(),
                tags: vec![],
                date: None,
                comment: None,
                rating: None,
                title: "Item 2".to_string(),
                type_: "movie".to_string(),
                poster: None,
            },
            DoubanItem {
                id: "2".to_string(),
                tags: vec![],
                date: None,
                comment: None,
                rating: None,
                title: "Item 2 duplicated".to_string(),
                type_: "movie".to_string(),
                poster: None,
            },
        ];
        let existing_ids = HashSet::from([String::from("1")]);
        let upload_ids = collect_refresh_upload_item_ids(&items, &existing_ids);
        assert!(upload_ids.contains("2"));
        assert!(!upload_ids.contains("1"));
        assert_eq!(upload_ids.len(), 1);
    }
}
