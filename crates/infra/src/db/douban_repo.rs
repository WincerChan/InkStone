use chrono::NaiveDate;
use sqlx::{PgPool, QueryBuilder, Row};
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

#[derive(Debug, Clone)]
pub struct DoubanMarkRecord {
    pub id: String,
    pub item_type: String,
    pub title: String,
    pub poster: Option<String>,
    pub date: NaiveDate,
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

pub async fn fetch_douban_marks_by_range(
    pool: &PgPool,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Vec<DoubanMarkRecord>, DoubanRepoError> {
    let rows = sqlx::query(
        r#"
        SELECT id, "type", title, poster, date
        FROM douban_items
        WHERE date IS NOT NULL
          AND date >= $1
          AND date < $2
        ORDER BY date ASC, id ASC
        "#,
    )
    .bind(start)
    .bind(end)
    .fetch_all(pool)
    .await?;

    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        items.push(DoubanMarkRecord {
            id: row.try_get("id")?,
            item_type: row.try_get("type")?,
            title: row.try_get("title")?,
            poster: row.try_get("poster")?,
            date: row.try_get("date")?,
        });
    }
    Ok(items)
}
