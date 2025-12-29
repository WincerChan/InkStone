use std::collections::HashMap;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::state::AppState;
use inkstone_core::domain::comments::{Comment, CommentThread};
use inkstone_infra::db::{find_discussion_by_post_id, list_comments};

const MAX_POST_ID_LEN: usize = 512;

#[derive(Debug, Deserialize)]
pub struct CommentsParams {
    pub post_id: Option<String>,
}

#[derive(Debug, Error)]
pub enum CommentsApiError {
    #[error("post_id is required")]
    MissingPostId,
    #[error("post_id is invalid")]
    InvalidPostId,
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

impl IntoResponse for CommentsApiError {
    fn into_response(self) -> axum::response::Response {
        let status = match self {
            CommentsApiError::MissingPostId | CommentsApiError::InvalidPostId => {
                StatusCode::BAD_REQUEST
            }
            CommentsApiError::DbUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            CommentsApiError::Db(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = Json(ErrorBody {
            error: self.to_string(),
        });
        (status, body).into_response()
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::build_comment_tree;
    use inkstone_infra::db::CommentRecord;

    #[test]
    fn build_comment_tree_nests_replies() {
        let records = vec![
            CommentRecord {
                discussion_id: "d1".to_string(),
                comment_id: "c1".to_string(),
                parent_id: None,
                comment_url: "https://github.com/owner/repo/discussions/1#discussioncomment-1"
                    .to_string(),
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
}
