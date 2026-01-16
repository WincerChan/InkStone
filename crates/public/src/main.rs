use inkstone_app::{run_public, AppError};

#[tokio::main]
async fn main() -> Result<(), AppError> {
    run_public().await
}
