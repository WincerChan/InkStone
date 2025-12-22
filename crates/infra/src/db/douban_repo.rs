use chrono::NaiveDate;
use sqlx::{PgPool, QueryBuilder};
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct DoubanItemRecord {
    pub id: String,
    pub item_type: String,
    pub title: String,
    pub poster: Option<String>,
    pub rating: Option<i16>,
    pub tags: Vec<String>,
    pub comment: Option<String>,
    pub date: Option<NaiveDate>,
}

#[derive(Debug, Error)]
pub enum DoubanRepoError {
    #[error("sqlx error: {0}")]
    Sqlx(#[from] sqlx::Error),
}

pub async fn upsert_douban_items(
    pool: &PgPool,
    items: &[DoubanItemRecord],
) -> Result<u64, DoubanRepoError> {
    if items.is_empty() {
        return Ok(0);
    }

    let mut builder = QueryBuilder::new(
        r#"
        INSERT INTO douban_items
            (id, "type", title, poster, rating, tags, comment, date)
        "#,
    );
    builder.push_values(items, |mut row, item| {
        row.push_bind(&item.id)
            .push_bind(&item.item_type)
            .push_bind(&item.title)
            .push_bind(&item.poster)
            .push_bind(item.rating)
            .push_bind(&item.tags)
            .push_bind(&item.comment)
            .push_bind(item.date);
    });
    builder.push(
        r#"
        ON CONFLICT ("type", id)
        DO UPDATE SET
            title = EXCLUDED.title,
            poster = EXCLUDED.poster,
            rating = EXCLUDED.rating,
            tags = EXCLUDED.tags,
            comment = EXCLUDED.comment,
            date = EXCLUDED.date
        "#,
    );

    let mut tx = pool.begin().await?;
    let result = builder.build().execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(result.rows_affected())
}

pub async fn insert_douban_items(
    pool: &PgPool,
    items: &[DoubanItemRecord],
) -> Result<u64, DoubanRepoError> {
    if items.is_empty() {
        return Ok(0);
    }

    let mut builder = QueryBuilder::new(
        r#"
        INSERT INTO douban_items
            (id, "type", title, poster, rating, tags, comment, date)
        "#,
    );
    builder.push_values(items, |mut row, item| {
        row.push_bind(&item.id)
            .push_bind(&item.item_type)
            .push_bind(&item.title)
            .push_bind(&item.poster)
            .push_bind(item.rating)
            .push_bind(&item.tags)
            .push_bind(&item.comment)
            .push_bind(item.date);
    });
    builder.push(r#"ON CONFLICT ("type", id) DO NOTHING"#);

    let mut tx = pool.begin().await?;
    let result = builder.build().execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(result.rows_affected())
}
