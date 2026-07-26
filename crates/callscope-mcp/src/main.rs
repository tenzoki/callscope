//! callscope-mcp — the stable-toolchain stdio MCP server for callscope.
//!
//! Loads the on-disk index once at start-up and serves capabilities C1–C8 as
//! eight MCP tools (see [`tools`]). Links no compiler internals, so it builds on
//! stable Rust; the nightly pin in `rust-toolchain.toml` is only for
//! `callscope-index`, and it builds this stable crate too.
//!
//! # Locating the index
//!
//! The server needs the **workspace root** — both to find the index and to run
//! the staleness fingerprint against the live sources. It is resolved, in order:
//!
//! 1. the first positional CLI argument, if given;
//! 2. otherwise the `CALLSCOPE_WORKSPACE` environment variable;
//! 3. otherwise the current working directory.
//!
//! The index and manifest are read from `<workspace>/.callscope/index.bin` and
//! `<workspace>/.callscope/manifest.json` (both JSON — see the on-disk-format
//! decision record). `callscope-index` writes them there.

mod state;
mod tools;

use std::path::PathBuf;
use std::sync::Arc;

use rmcp::transport::stdio;
use rmcp::ServiceExt;

use crate::state::{BoxError, IndexState};
use crate::tools::CallscopeServer;

/// Resolve the workspace root from argv[1], then `$CALLSCOPE_WORKSPACE`, then `.`.
fn resolve_workspace() -> PathBuf {
    if let Some(arg) = std::env::args().nth(1) {
        return PathBuf::from(arg);
    }
    if let Ok(env) = std::env::var("CALLSCOPE_WORKSPACE") {
        if !env.is_empty() {
            return PathBuf::from(env);
        }
    }
    PathBuf::from(".")
}

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    let workspace = resolve_workspace();
    // Diagnostics go to stderr so they never corrupt the stdio JSON-RPC stream.
    eprintln!(
        "callscope-mcp: loading index for workspace {}",
        workspace.display()
    );
    let state = IndexState::load(&workspace)?;
    eprintln!("callscope-mcp: index loaded; serving C1–C8 over stdio");

    let server = CallscopeServer::new(Arc::new(state));
    let running = server.serve(stdio()).await?;
    running.waiting().await?;
    Ok(())
}
