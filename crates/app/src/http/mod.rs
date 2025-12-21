pub mod middleware;
pub mod router;
pub mod routes;

use std::net::SocketAddr;

use thiserror::Error;
use tokio::net::TcpListener;

use crate::state::AppState;

#[derive(Debug, Error)]
pub enum HttpError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub async fn serve(addr: SocketAddr, state: AppState) -> Result<(), HttpError> {
    let router = router::build(state);
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, router).await?;
    Ok(())
}
