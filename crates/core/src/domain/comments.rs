use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Comment {
    pub id: String,
    pub url: String,
    pub source: String,
    pub author_login: Option<String>,
    pub author_url: Option<String>,
    pub author_avatar_url: Option<String>,
    pub body_html: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub replies: Vec<Comment>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommentThread {
    pub post_id: String,
    pub discussion_url: Option<String>,
    pub total: usize,
    pub comments: Vec<Comment>,
}
