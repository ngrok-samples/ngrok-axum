use axum::{routing::get, Router};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let app = Router::new().route("/", get(|| async { "hello from ngrok-axum!" }));

    let config = ngrok_axum::Config {
        url: Some("testregion.ngrok.app".to_string()),
        ..Default::default()
    };

    ngrok_axum::serve(app, config).await?;

    Ok(())
}
