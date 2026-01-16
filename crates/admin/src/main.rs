use inkstone_admin_core::{AppError, run};

#[tokio::main]
async fn main() -> Result<(), AppError> {
    run().await
}
