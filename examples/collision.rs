use axum::Router;
use ngrok_axum::{serve_many, Config, Endpoint};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    serve_many(vec![
        Endpoint {
            app: Router::new(),
            config: Config {
                url: Some("porttest.ngrok.app".to_string()),
                ..Default::default()
            },
        },
        Endpoint {
            app: Router::new(),
            config: Config {
                url: Some("porttest.ngrok.app".to_string()),
                ..Default::default()
            },
        },
    ])
    .await?;

    Ok(())
}
