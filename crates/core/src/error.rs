use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("invalid time range: {0}")]
    InvalidTimeRange(String),
    #[error("invalid slug: {0}")]
    InvalidSlug(String),
}
