use axum::body::Body;
use axum::http::Request;
use axum::middleware::Next;
use axum::response::Response;

use crate::mem_probe::{self, MemProbeContext};

const ADMIN_PATH_PREFIX: &str = "/v2/admin";

pub async fn record_admin_snapshot(request: Request<Body>, next: Next) -> Response {
    let path = request.uri().path().to_string();
    if !path.starts_with(ADMIN_PATH_PREFIX) {
        return next.run(request).await;
    }
    mem_probe::record_with_context(MemProbeContext::admin(&path, "before"));
    let response = next.run(request).await;
    mem_probe::record_with_context(MemProbeContext::admin(&path, "after"));
    response
}
