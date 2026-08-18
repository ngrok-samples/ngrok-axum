# ngrok-axum

Serve an Axum app through an ngrok tunnel with one call:

```rust
use axum::{routing::get, Router};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let app = Router::new().route("/", get(|| async { "hello" }));
    ngrok_axum::serve(app, ngrok_axum::Config::default()).await?;
    Ok(())
}
```

## Setup

1. Sign up at [ngrok.com](https://ngrok.com) and grab an authtoken from
   [the dashboard](https://dashboard.ngrok.com/authtokens).
2. Set it as an env var:
   ```
   NGROK_AUTHTOKEN=your_token_here
   ```
3. Add the dependency and call `serve`:
   ```toml
   [dependencies]
   ngrok-axum = "0.1"
   axum = "0.7"
   tokio = { version = "1", features = ["full"] }
   ```

You'll get a public URL bound to your ngrok account's default dev domain if you don't specify
one — see [`examples/basic.rs`](examples/basic.rs).

## Config

```rust
let config = ngrok_axum::Config {
    url: Some("your-reserved-domain.ngrok.app".to_string()), // None = account's default dev domain
    pooling: false,
    traffic_policy: Some(r#"
on_http_request:
  - actions:
      - type: basic-auth
        config:
          credentials:
            - "user:password123"
"#.to_string()),
    binding: None, // Some(ngrok_axum::Binding::Internal) etc.
};

ngrok_axum::serve(app, config).await?;
```

See [`examples/with_config.rs`](examples/with_config.rs) for a working example.

## Multiple endpoints

`serve_many` opens one endpoint per [`Endpoint`], each with its own `Router` and `Config`, all
on a single session, served concurrently:

```rust
use ngrok_axum::{serve_many, Config, Endpoint};

serve_many(vec![
    Endpoint {
        app: app_a,
        config: Config { url: Some("app.mycompany.ngrok.app".into()), ..Default::default() },
    },
    Endpoint {
        app: app_b,
        config: Config { url: Some("api.mycompany.ngrok.app".into()), ..Default::default() },
    },
])
.await?;
```

`serve(app, config)` is just `serve_many(vec![Endpoint { app, config }])` — the one-endpoint
case of the same function. See [`examples/multi.rs`](examples/multi.rs) (two distinct domains,
two distinct apps) and [`examples/pooling.rs`](examples/pooling.rs) (one domain, `pooling:
true`, two apps sharing it with load-balanced routing).

| Field | Type | Description |
|---|---|---|
| `url` | `Option<String>` | Reserved domain for this endpoint. `None` falls back to the account's default dev domain. |
| `pooling` | `bool` | Opt in to ngrok endpoint pooling — required if another endpoint on the same session would otherwise collide on the same domain. See [Collisions](#collisions) below. |
| `traffic_policy` | `Option<String>` | A raw [ngrok Traffic Policy](https://ngrok.com/docs/traffic-policy/) document (YAML or JSON) — the mechanism for auth, IP restrictions, header manipulation, webhook verification, and more. The granular builder methods on `ngrok-rust`'s `HttpTunnelBuilder` (`basic_auth`, `oauth`, `allow_cidr`, etc.) mirror ngrok's old "Edge Modules" system, superseded by Traffic Policy — use this instead. |
| `binding` | `Option<Binding>` | `Public` / `Internal` / `Kubernetes` ingress configuration. Not part of Traffic Policy (checked ngrok's actions reference directly — no equivalent exists), so it stays a standalone field. `Internal` requires `url` to end in `.internal`, enforced by ngrok itself (`ERR_NGROK_9029` if it doesn't). |

## Development

```
cargo build
cargo test
cargo run --example basic
```

See [`DESIGN.md`](DESIGN.md) for the design rationale, prior art check, and everything
confirmed empirically along the way.

## License

MIT OR Apache-2.0
