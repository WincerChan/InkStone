use std::collections::HashMap;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::warn;

use crate::http::middleware::public_token::{self, PublicTokenError};
use crate::state::AppState;
use inkstone_core::domain::comments::{Comment, CommentThread};
use inkstone_infra::db::{find_discussion_by_post_id, list_comments, list_discussions};

const MAX_POST_ID_LEN: usize = 512;
const MAPPING_TOKEN_PATH: &str = "/v2/comments/mapping";

#[derive(Debug, Deserialize)]
pub struct CommentsParams {
    pub post_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CommentsMappingParams {
    #[serde(rename = "inkstone_token")]
    pub inkstone_token: Option<String>,
}

#[derive(Debug, Error)]
pub enum CommentsApiError {
    #[error("post_id is required")]
    MissingPostId,
    #[error("post_id is invalid")]
    InvalidPostId,
    #[error("token is required")]
    MissingToken,
    #[error("token is invalid")]
    InvalidToken,
    #[error("token not configured")]
    TokenNotConfigured,
    #[error("db not configured")]
    DbUnavailable,
    #[error("db error: {0}")]
    Db(#[from] inkstone_infra::db::CommentsRepoError),
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
}

pub async fn get_comments(
    State(state): State<AppState>,
    Query(params): Query<CommentsParams>,
) -> Result<Json<CommentThread>, CommentsApiError> {
    let post_id = normalize_post_id(params.post_id)?;
    let pool = state.db.as_ref().ok_or(CommentsApiError::DbUnavailable)?;
    let discussion = find_discussion_by_post_id(pool, &post_id).await?;
    let Some(discussion) = discussion else {
        return Ok(Json(CommentThread {
            post_id,
            discussion_url: None,
            total: 0,
            comments: Vec::new(),
        }));
    };
    let records = list_comments(pool, &discussion.discussion_id).await?;
    let comments = build_comment_tree(&records);
    Ok(Json(CommentThread {
        post_id,
        discussion_url: Some(discussion.url),
        total: records.len(),
        comments,
    }))
}

#[derive(Debug, Serialize)]
pub struct CommentsMappingResponse {
    pub generated_at: String,
    pub total: usize,
    pub items: Vec<CommentsMappingItem>,
}

#[derive(Debug, Serialize)]
pub struct CommentsMappingItem {
    pub post_id: String,
    pub discussion_url: String,
    pub updated_at: String,
}

pub async fn get_comments_mapping(
    State(state): State<AppState>,
    Query(params): Query<CommentsMappingParams>,
) -> Result<Json<CommentsMappingResponse>, CommentsApiError> {
    verify_mapping_token(&state, params.inkstone_token.as_deref())?;
    let pool = state.db.as_ref().ok_or(CommentsApiError::DbUnavailable)?;
    let discussions = list_discussions(pool).await?;
    let items = discussions
        .into_iter()
        .map(|discussion| CommentsMappingItem {
            post_id: discussion.post_id,
            discussion_url: discussion.url,
            updated_at: discussion.updated_at.to_rfc3339(),
        })
        .collect::<Vec<_>>();
    Ok(Json(CommentsMappingResponse {
        generated_at: chrono::Utc::now().to_rfc3339(),
        total: items.len(),
        items,
    }))
}

fn normalize_post_id(value: Option<String>) -> Result<String, CommentsApiError> {
    let raw = value.unwrap_or_default();
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(CommentsApiError::MissingPostId);
    }
    if trimmed.len() > MAX_POST_ID_LEN || !trimmed.starts_with('/') {
        return Err(CommentsApiError::InvalidPostId);
    }
    if trimmed.chars().any(|ch| ch.is_whitespace()) {
        return Err(CommentsApiError::InvalidPostId);
    }
    Ok(trimmed.to_string())
}

fn build_comment_tree(records: &[inkstone_infra::db::CommentRecord]) -> Vec<Comment> {
    let mut nodes = Vec::with_capacity(records.len());
    let mut index = HashMap::new();
    for record in records {
        let comment = Comment {
            id: record.comment_id.clone(),
            url: record.comment_url.clone(),
            source: record.source.clone(),
            author_login: record.author_login.clone(),
            author_url: record.author_url.clone(),
            author_avatar_url: record.author_avatar_url.clone(),
            body_html: record.body_html.clone(),
            created_at: record.created_at,
            updated_at: record.updated_at,
            replies: Vec::new(),
        };
        let node_index = nodes.len();
        nodes.push(CommentNode {
            parent_id: record.parent_id.clone(),
            comment,
        });
        index.insert(record.comment_id.clone(), node_index);
    }

    let mut children: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    let mut roots = Vec::new();
    for (idx, node) in nodes.iter().enumerate() {
        if let Some(parent_id) = node.parent_id.as_ref() {
            if let Some(parent_idx) = index.get(parent_id) {
                children[*parent_idx].push(idx);
                continue;
            }
        }
        roots.push(idx);
    }

    let mut result = Vec::new();
    for idx in roots {
        result.push(build_comment_node(idx, &nodes, &children));
    }
    result
}

fn build_comment_node(
    idx: usize,
    nodes: &[CommentNode],
    children: &[Vec<usize>],
) -> Comment {
    let mut comment = nodes[idx].comment.clone();
    for child_idx in &children[idx] {
        comment.replies.push(build_comment_node(*child_idx, nodes, children));
    }
    comment
}

struct CommentNode {
    parent_id: Option<String>,
    comment: Comment,
}

fn verify_mapping_token(state: &AppState, token: Option<&str>) -> Result<(), CommentsApiError> {
    let token = token.ok_or(CommentsApiError::MissingToken)?;
    let secret = state
        .config
        .public_token_secret
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or(CommentsApiError::TokenNotConfigured)?;
    let path =
        public_token::extract_path_from_token(secret, token).map_err(map_token_error)?;
    if path != MAPPING_TOKEN_PATH {
        return Err(CommentsApiError::InvalidToken);
    }
    Ok(())
}

impl IntoResponse for CommentsApiError {
    fn into_response(self) -> axum::response::Response {
        warn!(error = %self, "comments api error");
        let status = match self {
            CommentsApiError::MissingPostId | CommentsApiError::InvalidPostId => {
                StatusCode::BAD_REQUEST
            }
            CommentsApiError::MissingToken => StatusCode::BAD_REQUEST,
            CommentsApiError::InvalidToken => StatusCode::UNAUTHORIZED,
            CommentsApiError::TokenNotConfigured => StatusCode::SERVICE_UNAVAILABLE,
            CommentsApiError::DbUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            CommentsApiError::Db(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = Json(ErrorBody {
            error: self.to_string(),
        });
        (status, body).into_response()
    }
}

fn map_token_error(err: PublicTokenError) -> CommentsApiError {
    match err {
        PublicTokenError::InvalidToken => CommentsApiError::InvalidToken,
        PublicTokenError::InvalidPath => CommentsApiError::InvalidToken,
    }
}

#[cfg(test)]
mod tests {
    use super::{get_comments_mapping, verify_mapping_token, CommentsApiError, MAPPING_TOKEN_PATH};
    use crate::http::middleware::public_token;
    use chrono::{TimeZone, Utc};

    use super::build_comment_tree;
    use axum::extract::{Query, State};
    use inkstone_infra::db::CommentRecord;
    use inkstone_runtime::config::PublicConfig;
    use std::sync::Arc;
    use crate::state::AppState;

    #[test]
    fn build_comment_tree_nests_replies() {
        let records = vec![
            CommentRecord {
                discussion_id: "d1".to_string(),
                comment_id: "c1".to_string(),
                parent_id: None,
                comment_url: "https://github.com/owner/repo/discussions/1#discussioncomment-1"
                    .to_string(),
                source: "github".to_string(),
                author_login: None,
                author_url: None,
                author_avatar_url: None,
                body_html: "root".to_string(),
                created_at: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
                updated_at: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            },
            CommentRecord {
                discussion_id: "d1".to_string(),
                comment_id: "c2".to_string(),
                parent_id: Some("c1".to_string()),
                comment_url: "https://github.com/owner/repo/discussions/1#discussioncomment-2"
                    .to_string(),
                source: "github".to_string(),
                author_login: None,
                author_url: None,
                author_avatar_url: None,
                body_html: "reply".to_string(),
                created_at: Utc.with_ymd_and_hms(2024, 1, 1, 0, 1, 0).unwrap(),
                updated_at: Utc.with_ymd_and_hms(2024, 1, 1, 0, 1, 0).unwrap(),
            },
        ];
        let tree = build_comment_tree(&records);
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].replies.len(), 1);
        assert_eq!(tree[0].replies[0].id, "c2");
    }

    fn build_state(secret: Option<&str>, db_configured: bool) -> AppState {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let index_dir = std::env::temp_dir().join(format!("inkstone-comments-{suffix}"));
        let _ = std::fs::create_dir_all(&index_dir);
        let config = PublicConfig {
            http_addr: "127.0.0.1:8080".parse().unwrap(),
            index_dir: index_dir.clone(),
            max_search_limit: 50,
            database_url: None,
            cookie_secret: Some("cookie".to_string()),
            stats_secret: Some("stats".to_string()),
            search_hash_secret: None,
            public_token_secret: secret.map(|value| value.to_string()),
            cors_allow_origins: Vec::new(),
            pulse_allowed_slds: Vec::new(),
        };
        let db = if db_configured {
            Some(inkstone_infra::db::connect_lazy("postgres://user:pass@localhost/db").unwrap())
        } else {
            None
        };
        AppState {
            config: Arc::new(config),
            search: Arc::new(inkstone_infra::search::SearchIndex::open_or_create(&index_dir).unwrap()),
            db,
        }
    }

    #[test]
    fn verify_mapping_token_requires_secret() {
        let state = build_state(None, false);
        let err = verify_mapping_token(&state, Some("token")).unwrap_err();
        assert!(matches!(err, CommentsApiError::TokenNotConfigured));
    }

    #[test]
    fn verify_mapping_token_rejects_invalid_token() {
        let state = build_state(Some("secret"), false);
        let err = verify_mapping_token(&state, Some("bad")).unwrap_err();
        assert!(matches!(err, CommentsApiError::InvalidToken));
    }

    #[test]
    fn verify_mapping_token_rejects_wrong_path() {
        let state = build_state(Some("secret"), false);
        let token = public_token::issue_token("secret", "/posts/hello/").unwrap();
        let err = verify_mapping_token(&state, Some(&token)).unwrap_err();
        assert!(matches!(err, CommentsApiError::InvalidToken));
    }

    #[test]
    fn verify_mapping_token_accepts_mapping_path() {
        let state = build_state(Some("secret"), false);
        let token = public_token::issue_token("secret", MAPPING_TOKEN_PATH).unwrap();
        verify_mapping_token(&state, Some(&token)).unwrap();
    }

    #[tokio::test]
    async fn comments_mapping_requires_db() {
        let state = build_state(Some("secret"), false);
        let token = public_token::issue_token("secret", MAPPING_TOKEN_PATH).unwrap();
        let params = super::CommentsMappingParams {
            inkstone_token: Some(token),
        };
        let result = get_comments_mapping(State(state), Query(params)).await;
        assert!(matches!(result.unwrap_err(), CommentsApiError::DbUnavailable));
    }
}
