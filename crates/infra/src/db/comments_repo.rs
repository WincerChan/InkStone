use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row, Transaction};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CommentsRepoError {
    #[error("sqlx error: {0}")]
    Sqlx(#[from] sqlx::Error),
}

#[derive(Debug, Clone)]
pub struct DiscussionRecord {
    pub post_id: String,
    pub discussion_id: String,
    pub number: i32,
    pub title: String,
    pub url: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CommentRecord {
    pub discussion_id: String,
    pub comment_id: String,
    pub parent_id: Option<String>,
    pub comment_url: String,
    pub author_login: Option<String>,
    pub author_url: Option<String>,
    pub author_avatar_url: Option<String>,
    pub body_html: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub async fn upsert_discussion(
    pool: &PgPool,
    record: &DiscussionRecord,
) -> Result<(), CommentsRepoError> {
    sqlx::query(
        r#"
        INSERT INTO comment_discussions (
            post_id,
            discussion_id,
            number,
            title,
            url,
            created_at,
            updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT (post_id)
        DO UPDATE SET
            discussion_id = EXCLUDED.discussion_id,
            number = EXCLUDED.number,
            title = EXCLUDED.title,
            url = EXCLUDED.url,
            created_at = EXCLUDED.created_at,
            updated_at = EXCLUDED.updated_at
        "#,
    )
    .bind(&record.post_id)
    .bind(&record.discussion_id)
    .bind(record.number)
    .bind(&record.title)
    .bind(&record.url)
    .bind(record.created_at)
    .bind(record.updated_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn replace_comments(
    pool: &PgPool,
    discussion_id: &str,
    comments: &[CommentRecord],
) -> Result<(), CommentsRepoError> {
    let mut tx = pool.begin().await?;
    delete_comments(&mut tx, discussion_id).await?;
    for comment in comments {
        insert_comment(&mut tx, comment).await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn find_discussion_by_post_id(
    pool: &PgPool,
    post_id: &str,
) -> Result<Option<DiscussionRecord>, CommentsRepoError> {
    let row = sqlx::query(
        r#"
        SELECT post_id, discussion_id, number, title, url, created_at, updated_at
        FROM comment_discussions
        WHERE post_id = $1
        "#,
    )
    .bind(post_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(map_discussion))
}

pub async fn find_discussion_by_discussion_id(
    pool: &PgPool,
    discussion_id: &str,
) -> Result<Option<DiscussionRecord>, CommentsRepoError> {
    let row = sqlx::query(
        r#"
        SELECT post_id, discussion_id, number, title, url, created_at, updated_at
        FROM comment_discussions
        WHERE discussion_id = $1
        "#,
    )
    .bind(discussion_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(map_discussion))
}

pub async fn list_discussions(pool: &PgPool) -> Result<Vec<DiscussionRecord>, CommentsRepoError> {
    let rows = sqlx::query(
        r#"
        SELECT post_id, discussion_id, number, title, url, created_at, updated_at
        FROM comment_discussions
        ORDER BY updated_at DESC
        "#,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(map_discussion).collect())
}

pub async fn list_comments(
    pool: &PgPool,
    discussion_id: &str,
) -> Result<Vec<CommentRecord>, CommentsRepoError> {
    let rows = sqlx::query(
        r#"
        SELECT discussion_id,
               comment_id,
               parent_id,
               comment_url,
               author_login,
               author_url,
               author_avatar_url,
               body_html,
               created_at,
               updated_at
        FROM comment_items
        WHERE discussion_id = $1
        ORDER BY created_at ASC
        "#,
    )
    .bind(discussion_id)
    .fetch_all(pool)
    .await?;
    let mut comments = Vec::with_capacity(rows.len());
    for row in rows {
        comments.push(CommentRecord {
            discussion_id: row.try_get("discussion_id")?,
            comment_id: row.try_get("comment_id")?,
            parent_id: row.try_get("parent_id")?,
            comment_url: row.try_get("comment_url")?,
            author_login: row.try_get("author_login")?,
            author_url: row.try_get("author_url")?,
            author_avatar_url: row.try_get("author_avatar_url")?,
            body_html: row.try_get("body_html")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        });
    }
    Ok(comments)
}

async fn delete_comments(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    discussion_id: &str,
) -> Result<(), CommentsRepoError> {
    sqlx::query(
        r#"
        DELETE FROM comment_items
        WHERE discussion_id = $1
        "#,
    )
    .bind(discussion_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn insert_comment(
    tx: &mut Transaction<'_, sqlx::Postgres>,
    comment: &CommentRecord,
) -> Result<(), CommentsRepoError> {
    sqlx::query(
        r#"
        INSERT INTO comment_items (
            discussion_id,
            comment_id,
            parent_id,
            comment_url,
            author_login,
            author_url,
            author_avatar_url,
            body_html,
            created_at,
            updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        "#,
    )
    .bind(&comment.discussion_id)
    .bind(&comment.comment_id)
    .bind(&comment.parent_id)
    .bind(&comment.comment_url)
    .bind(&comment.author_login)
    .bind(&comment.author_url)
    .bind(&comment.author_avatar_url)
    .bind(&comment.body_html)
    .bind(comment.created_at)
    .bind(comment.updated_at)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn map_discussion(row: sqlx::postgres::PgRow) -> DiscussionRecord {
    DiscussionRecord {
        post_id: row.get("post_id"),
        discussion_id: row.get("discussion_id"),
        number: row.get("number"),
        title: row.get("title"),
        url: row.get("url"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}
