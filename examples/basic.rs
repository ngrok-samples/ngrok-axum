use axum::{routing::get, Router};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let app = Router::new().route("/", get(|| async { "hello from ngrok-axum!" }));

    ngrok_axum::serve(app, ngrok_axum::Config::default()).await?;

    Ok(())
}
