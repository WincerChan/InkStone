use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

const MAX_SEARCH_QUERY_LEN: usize = 512;

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

pub async fn enforce_search_query_length(req: Request, next: Next) -> Response {
    if is_query_too_long(req.uri().query()) {
        let body = ErrorBody {
            error: format!(
                "query string too long (max {} chars)",
                MAX_SEARCH_QUERY_LEN
            ),
        };
        return (StatusCode::URI_TOO_LONG, axum::Json(body)).into_response();
    }
    next.run(req).await
}

fn is_query_too_long(query: Option<&str>) -> bool {
    query.map_or(false, |value| value.len() > MAX_SEARCH_QUERY_LEN)
}

#[cfg(test)]
mod tests {
    use super::{is_query_too_long, MAX_SEARCH_QUERY_LEN};

    #[test]
    fn query_length_within_limit() {
        let query = "a".repeat(MAX_SEARCH_QUERY_LEN);
        assert!(!is_query_too_long(Some(&query)));
    }

    #[test]
    fn query_length_exceeds_limit() {
        let query = "a".repeat(MAX_SEARCH_QUERY_LEN + 1);
        assert!(is_query_too_long(Some(&query)));
    }
}
