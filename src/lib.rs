//! Serve an Axum app through an ngrok tunnel with one call.
//!
//! `axum::Server` was removed in axum 0.7, which broke the one-liner
//! `ngrok-rust` used to offer (see <https://github.com/ngrok/ngrok-rust/issues/136>).
//! The fix upstream was to replace the example with a ~30-line manual
//! connection-serving loop, not to restore a one-liner. [`serve`] is that
//! restored one-liner.
//!
//! ```no_run
//! use axum::{routing::get, Router};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//!     let app = Router::new().route("/", get(|| async { "hello" }));
//!     ngrok_axum::serve(app, ngrok_axum::Config::default()).await?;
//!     Ok(())
//! }
//! ```

use std::{collections::HashMap, io, net::SocketAddr};

use axum::Router;
use axum_core::BoxError;
use futures::stream::TryStreamExt;
use hyper::{body::Incoming, Request};
use hyper_util::{rt::TokioExecutor, server};
use ngrok::{config::HttpTunnelBuilder, prelude::*, Session};
use tower::{util::ServiceExt, Service};

/// Ingress binding for the endpoint. Not part of ngrok's Traffic Policy —
/// confirmed by checking the Traffic Policy actions reference directly, so
/// this stays a standalone field rather than folded into `traffic_policy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Binding {
    Public,
    /// Requires `url` to end in `.internal` — enforced by ngrok itself
    /// (`ERR_NGROK_9029` if it doesn't), not something this crate validates.
    Internal,
    Kubernetes,
}

impl Binding {
    fn as_str(self) -> &'static str {
        match self {
            Binding::Public => "public",
            Binding::Internal => "internal",
            Binding::Kubernetes => "kubernetes",
        }
    }
}

/// Configuration for a single ngrok endpoint.
///
/// Deliberately minimal, same reasoning as the `ngrok-nextjs` sibling
/// project's config: the ~15 granular per-module builder methods on
/// `HttpTunnelBuilder` (`basic_auth`, `oauth`, header manipulation, CIDR
/// restrictions, etc.) mirror ngrok's old "Edge Modules" system, superseded
/// by Traffic Policy — confirmed against ngrok's actions reference, not
/// assumed. Use `traffic_policy` for all of that instead.
#[derive(Debug, Default, Clone)]
pub struct Config {
    /// Reserved domain for this endpoint. `None` falls back to the
    /// account's default dev domain.
    pub url: Option<String>,
    /// Opt in to ngrok endpoint pooling — required if another endpoint on
    /// the same session would otherwise land on the same domain. ngrok does
    /// not reject or warn on that collision by default: confirmed live
    /// against the real SDK that two listeners opened with no domain and no
    /// pooling both succeed silently, returning the identical URL, with
    /// only the most recently opened one actually receiving traffic.
    pub pooling: bool,
    /// Raw ngrok Traffic Policy document (YAML or JSON), passed straight
    /// through. See <https://ngrok.com/docs/traffic-policy/>.
    pub traffic_policy: Option<String>,
    pub binding: Option<Binding>,
}

/// One app bound to one endpoint, for [`serve_many`]. The same `Router` can
/// be cloned across multiple `Endpoint`s to pool one app behind several
/// listeners.
pub struct Endpoint {
    pub app: Router,
    pub config: Config,
}

/// Opens a single ngrok HTTP endpoint per `config` and serves `app` on it.
/// Shorthand for the one-endpoint case of [`serve_many`].
pub async fn serve(app: Router, config: Config) -> Result<(), BoxError> {
    serve_many(vec![Endpoint { app, config }]).await
}

/// Opens one ngrok HTTP endpoint per entry in `endpoints`, all on a single
/// session, and serves each endpoint's app concurrently until every
/// listener closes or a connection-level error occurs.
pub async fn serve_many(endpoints: Vec<Endpoint>) -> Result<(), BoxError> {
    validate_url_groups(&endpoints)?;

    let session = Session::builder().authtoken_from_env().connect().await?;

    let mut tasks = Vec::with_capacity(endpoints.len());
    for endpoint in endpoints {
        let mut builder = session.http_endpoint();
        configure(&mut builder, &endpoint.config);

        let mut listener = builder.listen().await?;
        println!("Ingress established at: {:?}", listener.url());

        let app = endpoint.app;
        tasks.push(tokio::spawn(async move {
            let mut make_service = app.into_make_service_with_connect_info::<SocketAddr>();

            while let Some(conn) = listener.try_next().await? {
                let remote_addr = conn.remote_addr();
                let tower_service = make_service
                    .call(remote_addr)
                    .await
                    .unwrap_or_else(|err: std::convert::Infallible| match err {});

                tokio::spawn(async move {
                    let hyper_service =
                        hyper::service::service_fn(move |request: Request<Incoming>| {
                            tower_service.clone().oneshot(request)
                        });

                    if let Err(err) = server::conn::auto::Builder::new(TokioExecutor::new())
                        .serve_connection_with_upgrades(conn, hyper_service)
                        .await
                    {
                        eprintln!("failed to serve connection: {err:#}");
                    }
                });
            }

            Ok::<(), BoxError>(())
        }));
    }

    for task in tasks {
        task.await??;
    }

    Ok(())
}

fn configure(builder: &mut HttpTunnelBuilder, config: &Config) {
    if let Some(url) = &config.url {
        builder.domain(url.clone());
    }
    if config.pooling {
        builder.pooling_enabled(true);
    }
    if let Some(policy) = &config.traffic_policy {
        builder.traffic_policy(policy.clone());
    }
    if let Some(binding) = config.binding {
        builder.binding(binding.as_str());
    }
}

/// ngrok does not reject or warn when two listeners in the same session end
/// up on the same public url — it silently lets the most recently opened
/// listener win, and the other becomes unreachable with zero indication of
/// why. Confirmed live against the real SDK, isolated from this crate's own
/// code (a standalone program opening two listeners with no domain, no
/// pooling, saw both succeed with the identical URL, no error either time).
/// An endpoint with no explicit `url` falls back to the account's default
/// dev domain, so two url-less endpoints collide there just as surely as
/// two hardcoding the same string. Either case is fine *if* every endpoint
/// sharing that url opts into `pooling: true` — otherwise it's the
/// silent-failure footgun, so fail fast instead.
///
/// Note this is a different failure mode than a *cross-session* claim on a
/// domain (another running agent, or a dashboard-configured Cloud Endpoint)
/// — ngrok rejects that loudly on its own (`ERR_NGROK_334`), confirmed live;
/// this guard only covers the silent same-session case ngrok doesn't catch.
fn validate_url_groups(endpoints: &[Endpoint]) -> Result<(), BoxError> {
    let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, endpoint) in endpoints.iter().enumerate() {
        let key = endpoint.config.url.clone().unwrap_or_default();
        groups.entry(key).or_default().push(i);
    }

    for (url, indices) in &groups {
        if indices.len() <= 1 {
            continue;
        }
        if indices.iter().all(|&i| endpoints[i].config.pooling) {
            continue;
        }

        let label = if url.is_empty() {
            "the account's default dev domain".to_string()
        } else {
            format!("\"{url}\"")
        };
        return Err(Box::new(io::Error::other(format!(
            "Endpoints at indices {indices:?} would all share {label}. Without endpoint \
             pooling, only the most recently opened listener actually receives traffic — the \
             rest go silently unreachable. Set `pooling: true` on each of these endpoints to \
             share it intentionally, or give each its own url."
        ))));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint(url: Option<&str>, pooling: bool) -> Endpoint {
        Endpoint {
            app: Router::new(),
            config: Config {
                url: url.map(String::from),
                pooling,
                ..Default::default()
            },
        }
    }

    #[test]
    fn binding_as_str_matches_ngrok_valid_values() {
        assert_eq!(Binding::Public.as_str(), "public");
        assert_eq!(Binding::Internal.as_str(), "internal");
        assert_eq!(Binding::Kubernetes.as_str(), "kubernetes");
    }

    #[test]
    fn single_endpoint_never_errors() {
        assert!(validate_url_groups(&[endpoint(None, false)]).is_ok());
    }

    #[test]
    fn distinct_urls_never_error() {
        let endpoints = [endpoint(Some("a.ngrok.app"), false), endpoint(Some("b.ngrok.app"), false)];
        assert!(validate_url_groups(&endpoints).is_ok());
    }

    #[test]
    fn two_url_less_endpoints_without_pooling_errors() {
        let endpoints = [endpoint(None, false), endpoint(None, false)];
        let err = validate_url_groups(&endpoints).unwrap_err();
        assert!(err.to_string().contains("account's default dev domain"));
    }

    #[test]
    fn two_endpoints_sharing_a_url_without_pooling_errors() {
        let endpoints = [endpoint(Some("a.ngrok.app"), false), endpoint(Some("a.ngrok.app"), false)];
        let err = validate_url_groups(&endpoints).unwrap_err();
        assert!(err.to_string().contains("a.ngrok.app"));
    }

    #[test]
    fn pooling_on_every_colliding_entry_does_not_error() {
        let endpoints = [endpoint(None, true), endpoint(None, true)];
        assert!(validate_url_groups(&endpoints).is_ok());
    }

    #[test]
    fn pooling_on_only_some_colliding_entries_still_errors() {
        let endpoints = [endpoint(None, true), endpoint(None, false)];
        assert!(validate_url_groups(&endpoints).is_err());
    }
}
