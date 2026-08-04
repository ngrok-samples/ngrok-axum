# ngrok + Axum Integration — Design Sketch (v0)

## Research: why Axum

Second integration in the "ngrok SDK + popular framework" series, after
[`ngrok-nextjs`](../ngrok-nextjs) (JS SDK + Next.js). This one targets the Rust SDK.

**Popularity**: Axum has ~352M all-time crates.io downloads vs. Actix-web's ~70M — roughly a
10x gap, and Axum's recent download volume (~87M) is also ~10x Actix's (~8.6M), so the gap is
widening, not closing. Rocket and Warp trail well behind both. Current (2026) expert consensus
treats Axum as the default first choice for new Rust web services.

**Relevance**: Axum is built by the Tokio team, and ngrok's own Rust SDK (`ngrok-rust`) is
built to compose with that exact stack — its listeners implement Tokio's `AsyncRead`/
`AsyncWrite` and are hyper-compatible. More directly: `ngrok-rust` ships a dedicated, optional
**`axum` Cargo feature** (`ngrok = { version = "...", features = ["axum"] }`) and a maintained
`examples/axum.rs` in its own repo — ngrok has already picked Axum as *the* framework it
demonstrates against. This isn't a cold start the way Next.js was.

## The problem — confirmed by reading the SDK's own history, not assumed

Unlike Next.js, there's no black-box dev command to wrap. A Rust binary is code the developer
already wrote and fully owns — `cargo run` isn't hiding anything the way `next dev` does.
So the shape of "an integration" here is fundamentally different: not a CLI wrapper spawning
a child process, but making what the developer writes in their own `main.rs` shorter and less
fragile.

**It used to be one line.** An older example (axum 0.6, ngrok crate 0.13.1):

```rust
let listener = ngrok::Session::builder()
    .authtoken_from_env()
    .connect().await?
    .http_endpoint()
    .listen().await?;

axum::Server::builder(listener)
    .serve(app.into_make_service())
    .await?;
```

**Axum 0.7 removed `axum::Server`** in favor of `axum::serve()`, which doesn't accept an
ngrok listener directly. A real user hit this and filed it:
[ngrok-rust#136](https://github.com/ngrok/ngrok-rust/issues/136) — `error[E0433]: could not
find 'Server' in 'axum'`, closed without restoring a one-liner. The **current**
`examples/axum.rs` in the ngrok-rust repo (crate v0.19.0, axum 0.7.4) replaced it with ~30
lines: a manual `while let Some(conn) = listener.try_next().await?` loop, constructing a
`tower::Service` per connection via `make_service.call(remote_addr)`, wrapping it in a
`hyper::service::service_fn`, and manually driving it with
`hyper_util::server::conn::auto::Builder::new(TokioExecutor::new())
.serve_connection_with_upgrades(...)` inside a spawned task.

So the real, confirmed problem: **the ergonomics regressed and never recovered**, because the
fix was "update the example to the new verbose pattern," not "give users back a one-liner."
Every consumer of `ngrok-rust` + Axum today either copies that ~30-line block or writes their
own equivalent. That's a real, current gap — not a hypothetical one, and not something
Traffic Policy or any other ngrok feature addresses, since it's pure Rust/async-runtime
plumbing, unrelated to endpoint configuration.

## Shape of the solution: a library crate, not a CLI wrapper

This is the same kind of foundational fork "why not a Next.js plugin" was for the previous
project, and it resolves the opposite way here:

- **Next.js**: `next dev` is an opaque CLI command the developer doesn't own → a CLI wrapper
  spawning it as a child process was the only way in.
- **Axum**: the developer already owns `main.rs` and the whole process → wrapping a "dev
  command" makes no sense; there isn't one to wrap. The integration point is a **library
  crate** imported directly into that `main.rs`, the same way `ngrok-rust` itself already is.

Working name: `ngrok-axum` (crates.io). Core API sketch:

```rust
use axum::{Router, routing::get};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let app = Router::new().route("/", get(|| async { "hello" }));

    ngrok_axum::serve(app, ngrok_axum::Config::default()).await?;
    Ok(())
}
```

`ngrok_axum::serve()` internally does the `Session::builder()` → `http_endpoint()` →
`listen()` → connection-loop dance that's currently ~30 lines of copy-pasted boilerplate,
restoring the one-liner ergonomics that axum 0.6-era code had — this is the single highest-value
thing this crate does, and it's fixing a regression, not adding new capability.

## Config surface — same conclusions as the JS design, translated to Rust idioms

The JS design (`ngrok-nextjs`) already did the legwork on which SDK options are worth wrapping
directly vs. deferring to Traffic Policy — that reasoning transfers directly, since
`ngrok-rust`'s `HttpTunnelBuilder` exposes the *same* set of methods (confirmed by reading the
current `examples/axum.rs`, which lists them all, commented out, as a menu):
`allow_cidr`/`deny_cidr`, `basic_auth`, `circuit_breaker`, `compression`, `oauth`/`oidc`,
`proxy_proto`, `request_header`/`response_header`, `scheme`, `webhook_verification`, etc. —
this is a near-exact mirror of the JS `HttpListenerBuilder`, as expected since both wrap the
same ngrok agent core.

So `ngrok_axum::Config` should be similarly minimal:

```rust
pub struct Config {
    pub url: Option<String>,        // reserved domain; None = account default dev domain
    pub pooling: bool,
    pub traffic_policy: Option<String>,  // raw YAML/JSON, passed straight to .traffic_policy()
    pub binding: Option<Binding>,   // Public | Internal | Kubernetes — see note below
}
```

- `url`/domain, `pooling`, `traffic_policy`: same reasoning as the JS design — the granular
  per-module builder methods (`basic_auth`, `oauth`, header manipulation, CIDR restrictions,
  etc.) mirror what ngrok calls "Edge Modules," superseded by Traffic Policy. No need to
  re-verify that conclusion; it was already confirmed against ngrok's actual Traffic Policy
  actions reference for the JS project and applies identically here.
- `binding`: confirmed in the JS project to have **no** Traffic Policy equivalent (checked the
  actions reference directly — "Forward Internal" routes *to* an internal endpoint, doesn't
  declare one), so it stays a standalone field here too, for the same reason.
- Loading: env vars (`NGROK_AUTHTOKEN`, `NGROK_DOMAIN` — already idiomatic here, since
  `authtoken_from_env()` already exists in the SDK) plus a `Config::builder()` for anything
  set in code. No direct Rust equivalent of `ngrok.config.ts` is needed — Rust code *is* the
  config file, since the developer already owns `main.rs`.

## Confirmed since first draft

- **Multi-endpoint / pooling collision reproduces identically in Rust.** Wrote a minimal
  standalone Rust program (`ngrok::Session::builder()...http_endpoint().listen()`, called
  twice on the same session, no domain, no pooling) and ran it live against the real SDK
  (v0.19.0). Same result as the JS SDK: both calls succeeded, no error either time, both
  returned the identical URL (`unbearded-helicoidally-flor.ngrok-free.dev`). Confirms this is
  an ngrok platform/backend behavior, not a JS-binding-specific quirk — `ngrok-axum` needs the
  same collision guard the JS project built, not a lighter version of it.
- **Naming is free.** Checked crates.io directly: neither `ngrok-axum` nor `axum-ngrok` exists.
  Broader search for "ngrok" on crates.io turned up `cargo-ngrok` (unrelated — a trace-driven
  dev tool), `ngrok-wrapper` (unofficial, wraps the old downloaded `ngrok` CLI binary directly,
  not the `ngrok-rust` SDK — same relationship as the old npm `ngrok` package had to
  `@ngrok/ngrok`), and `cargo-doc-ngrok` (ngrok's own official cargo subcommand for serving
  docs — unrelated use case, but useful precedent that ngrok already ships cargo subcommands
  when that shape fits). No real competing or adjacent prior art for an Axum-specific wrapper.
- **Toolchain note for whoever builds this**: `ngrok` v0.19.0's dependency tree (via
  `hyper-http-proxy` → `hyper-rustls` → `rustls-native-certs` → `security-framework`, and via
  `tokio-retry` → `rand` → `getrandom`) requires Rust 1.85+ (edition2024). Confirmed by hitting
  this directly — building against Homebrew's then-installed 1.84.1 failed outright, and it's
  not fixable by pinning transitive versions since these are hard minimum-version requirements,
  not loose ranges. `rustup update stable` resolved it (1.97.1).

## v0 scaffold built and live-verified

`ngrok-axum` v0.1.0: `Config { url, pooling, traffic_policy, binding }` and a `serve(app,
config)` function that collapses the current ~30-line manual connection-serving loop back
into one call, matching the design above. `cargo test` passes (unit test + doc test), and the
`with_config` example was verified against a real tunnel — real request, real response from
the actual Axum handler (`"hello from ngrok-axum!"`, `200`), not just a successful compile.

**A third, distinct collision mode surfaced during that live test, not previously seen in the
JS project**: attempting to open an agent-based `http_endpoint()` listener on
`testregion.ngrok.app` while that domain had a **persistent Cloud Endpoint** configured (a
dashboard-managed, always-on config, independent of any connected agent) failed loudly —
`ERR_NGROK_334: "already online... stop your existing endpoint first, or start both endpoints
with --pooling-enabled"` — and curling the domain in the meantime returned ngrok's own default
Cloud Endpoint placeholder page, not our app. This is different from the same-session
silent-collision footgun above: a **cross-session** claim on a domain (whether from a Cloud
Endpoint or another running agent) is rejected clearly, not silently. Resolved by removing the
conflicting Cloud Endpoint; worth documenting in the README as a real error users may hit,
since the fix (stop the other endpoint, or use `pooling: true` on both) is non-obvious from
the error text alone otherwise.

## Multi-endpoint + collision guard: built and live-verified

`Endpoint { app: Router, config: Config }` plus `serve_many(endpoints: Vec<Endpoint>)` —
`serve()` is now just `serve_many(vec![Endpoint { app, config }])`, the one-endpoint case of
the same function, mirroring how the JS project unified its single/multi paths. A
`validate_url_groups` function (7 unit tests, same shape as the JS project's
`validateUrlGroups`) runs before any session is opened.

All three scenarios confirmed live, using both of the user's reserved domains
(`testregion.ngrok.app`, `porttest.ngrok.app`):

- **Distinct domains, two different apps**: both endpoints opened successfully, each domain
  correctly routed to its own app (`examples/multi.rs`).
- **Same domain, no pooling**: `serve_many` rejects immediately with a clear error naming both
  indices and the shared URL — confirmed this happens *before* any network call, since it
  still failed correctly even with no `NGROK_AUTHTOKEN` set at all (`examples/collision.rs`).
- **Same domain, `pooling: true` on both**: succeeds, and repeated requests genuinely
  alternated between the two apps' responses — real load-balanced routing, not just "no error"
  (`examples/pooling.rs`).

## Open questions

- **`cargo generate` template**: a scaffolding template (new project, ngrok already wired in)
  might be worth pairing with the library crate, given Rust has no equivalent of retrofitting
  a wrapper onto an *existing* arbitrary project the way `npx @ngrok/nextjs dev` can.
- **Distribution**: same open question as the JS project — value is contingent on this showing
  up in `ngrok-rust`'s own README/examples, not just existing as a separate crate no one finds.
