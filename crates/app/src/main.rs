#[tokio::main]
async fn main() -> Result<(), inkstone_app::AppError> {
    inkstone_app::run_public().await
}
