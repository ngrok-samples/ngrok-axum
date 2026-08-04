use axum::{routing::get, Router};
use ngrok_axum::{serve_many, Config, Endpoint};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let app_a = Router::new().route("/", get(|| async { "app A" }));
    let app_b = Router::new().route("/", get(|| async { "app B" }));

    serve_many(vec![
        Endpoint {
            app: app_a,
            config: Config {
                url: Some("testregion.ngrok.app".to_string()),
                ..Default::default()
            },
        },
        Endpoint {
            app: app_b,
            config: Config {
                url: Some("porttest.ngrok.app".to_string()),
                ..Default::default()
            },
        },
    ])
    .await?;

    Ok(())
}
