//! Sonarium — a sound-engineering MCP server: agents author symbolic synthesis
//! graphs; the server renders them deterministically and feeds back analysis.
//!
//! Two transports:
//! - **stdio** (default) — for clients that spawn the binary directly.
//! - **streamable HTTP** (`--http [addr]` or `SONARIUM_TRANSPORT=http`) — for
//!   connecting any networked MCP client.

use std::path::PathBuf;
use std::sync::Arc;

use rmcp::{
    ServiceExt,
    transport::{
        stdio,
        streamable_http_server::{
            StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
        },
    },
};

use sonarium::server::{Sonarium, rehydrate};
use sonarium::service;
use sonarium::session::Store;

/// Default HTTP bind address.
const DEFAULT_BIND: &str = "127.0.0.1:8787";

/// Selected transport.
enum Transport {
    Stdio,
    Http(String),
}

/// Resolve the working directory for rendered artifacts. `SONARIUM_WORKDIR`
/// overrides (point it at your game's assets); otherwise a stable per-user
/// `~/.sonarium/sounds`, falling back to a temp dir only when no home exists.
fn working_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("SONARIUM_WORKDIR") {
        return PathBuf::from(p);
    }
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        return PathBuf::from(home).join(".sonarium").join("sounds");
    }
    std::env::temp_dir().join("sonarium")
}

/// Pick the transport from CLI args / env.
/// `--http [addr]`, or `SONARIUM_TRANSPORT=http` (+ optional `SONARIUM_BIND`).
fn transport() -> Transport {
    let args: Vec<String> = std::env::args().collect();
    if let Some(pos) = args.iter().position(|a| a == "--http") {
        let addr = args
            .get(pos + 1)
            .filter(|a| !a.starts_with('-'))
            .cloned()
            .or_else(|| std::env::var("SONARIUM_BIND").ok())
            .unwrap_or_else(|| DEFAULT_BIND.to_string());
        return Transport::Http(addr);
    }
    if std::env::var("SONARIUM_TRANSPORT").as_deref() == Ok("http") {
        let addr = std::env::var("SONARIUM_BIND").unwrap_or_else(|_| DEFAULT_BIND.to_string());
        return Transport::Http(addr);
    }
    Transport::Stdio
}

const HELP: &str = "sonarium — a sound-engineering MCP server driven by tool calls.

USAGE:
    sonarium                       run the MCP server over stdio (for clients that spawn it)
    sonarium --http [ADDR]         run the streamable-HTTP MCP server (default 127.0.0.1:8787, endpoint /mcp)
    sonarium service install       install + start the background daemon (launchd / systemd --user)
             [--bind ADDR] [--workdir DIR]
    sonarium service status        show daemon state and log locations
    sonarium service uninstall     stop + remove the daemon
    sonarium --version             print the version

ENVIRONMENT:
    SONARIUM_WORKDIR    where renders/exports land (default ~/.sonarium/sounds) — point at your game's assets
    SONARIUM_BIND       HTTP bind address (with SONARIUM_TRANSPORT=http)
    RUST_LOG            log filter (logs go to stderr)";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Subcommands / flags that don't start the server.
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(|s| s.as_str()) {
        Some("service") => std::process::exit(service::run(&args[2..])),
        Some("--version") | Some("-V") => {
            println!("sonarium {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Some("--help") | Some("-h") => {
            println!("{HELP}");
            return Ok(());
        }
        _ => {}
    }

    // Logs go to stderr so they never corrupt the stdio JSON-RPC stream.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let dir = working_dir();
    let store = Arc::new(Store::new(dir.clone())?);
    // Rebuild the index from graphs persisted in a previous run so a restarted
    // server still sees earlier sounds (and banks).
    let restored = rehydrate(&store);
    tracing::info!(workdir = %dir.display(), restored, "sonarium starting");

    match transport() {
        Transport::Stdio => {
            let service = Sonarium::new(store).serve(stdio()).await?;
            service.waiting().await?;
        }
        Transport::Http(addr) => serve_http(store, &addr).await?,
    }
    Ok(())
}

/// Serve over streamable HTTP at `addr`, mounting the MCP endpoint at `/mcp`.
/// A fresh handler is created per session; all sessions share the same on-disk
/// store, so sounds authored in one connection are visible in another.
async fn serve_http(store: Arc<Store>, addr: &str) -> anyhow::Result<()> {
    let service = StreamableHttpService::new(
        move || Ok(Sonarium::new(store.clone())),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    );
    let router = axum::Router::new().nest_service("/mcp", service);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let bound = listener.local_addr()?;
    tracing::info!("sonarium MCP listening at http://{bound}/mcp");
    axum::serve(listener, router).await?;
    Ok(())
}
