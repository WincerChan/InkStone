use inkstone_app::{run_with_mode_override, AppError, Mode};

#[tokio::main]
async fn main() -> Result<(), AppError> {
    run_with_mode_override(Mode::Admin).await
}
