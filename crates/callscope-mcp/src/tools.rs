//! The MCP wire layer: the eight tools (C1–C8), their JSON-schema'd inputs, and
//! the `ServerHandler` that exposes them over stdio.
//!
//! Every tool follows the same three-step shape and nothing else lives here:
//! 1. run the staleness check ([`IndexState::compute_stale`]) — on *every* call,
//!    per Q6;
//! 2. resolve the symbol name(s) via C1, handing back the candidate set instead
//!    of guessing when a name is ambiguous (Q3);
//! 3. serialize the resulting [`Envelope`] to a compact JSON string and return it
//!    as tool text content.
//!
//! The answer logic itself is in [`crate::state`]; this module only marshals
//! arguments in and JSON out.

use std::sync::Arc;

use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerInfo};
use rmcp::schemars::{self, JsonSchema};
use rmcp::{tool, tool_handler, tool_router, ErrorData, ServerHandler};
use serde::Deserialize;

use callscope_core::query::Direction;
use callscope_core::{Envelope, StaleInfo, SymbolId};

use crate::state::{
    envelope_to_json, IndexState, DEFAULT_GRAPH_DEPTH, DEFAULT_LIMIT, DEFAULT_MAX_DEPTH,
    DEFAULT_MAX_PATHS, DEFAULT_NODE_LIMIT,
};

const INSTRUCTIONS: &str = "\
callscope answers compiler-grounded change-impact questions about a Rust workspace \
from a pre-built on-disk index. Every answer is an Envelope carrying uncertainty \
flags: `stale` (the index no longer matches the sources on disk — re-index before \
trusting the answer), `over_approximated` (the walk crossed a dyn-dispatch edge, so \
the set is a superset — read it as \"any workspace implementor\"), `truncated`+`total` \
(the list was capped; `total` is the true count), and `boundary_applies` (the walk \
reached the edge of your workspace crates, which v1 does not descend past). \
Symbol-taking tools accept a name or fragment; if it is ambiguous they return the \
candidate set instead of guessing — disambiguate by passing the exact fully-qualified \
path or the numeric symbol id.";

/// Direction argument for reachability (C3), serialized as `\"forward\"` /
/// `\"backward\"` on the wire.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum DirectionArg {
    /// What the symbol can reach (its transitive callees).
    #[default]
    Forward,
    /// What can reach the symbol (its transitive callers).
    Backward,
}

impl From<DirectionArg> for Direction {
    fn from(d: DirectionArg) -> Self {
        match d {
            DirectionArg::Forward => Direction::Forward,
            DirectionArg::Backward => Direction::Backward,
        }
    }
}

// ── tool input schemas ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ResolveArgs {
    /// A symbol name or path fragment to search for (case-insensitive substring).
    pub name: String,
    /// Maximum candidates to return.
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SymbolArgs {
    /// A symbol name, fully-qualified path, or numeric id. If a bare fragment is
    /// ambiguous, the candidate set is returned instead of an answer.
    pub symbol: String,
    /// Maximum results to return.
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReachabilityArgs {
    /// A symbol name, fully-qualified path, or numeric id.
    pub symbol: String,
    /// `forward` (callees) or `backward` (callers). Defaults to `forward`.
    pub direction: Option<DirectionArg>,
    /// Maximum results to return.
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CallPathsArgs {
    /// The path's start symbol (name, fully-qualified path, or numeric id).
    pub from: String,
    /// The path's end symbol (name, fully-qualified path, or numeric id).
    pub to: String,
    /// Maximum edges per enumerated path.
    pub max_depth: Option<usize>,
    /// Maximum number of paths to enumerate.
    pub max_paths: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct NeighborhoodArgs {
    /// The focus symbol (name, fully-qualified path, or numeric id).
    pub symbol: String,
    /// Undirected radius (hops) around the focus.
    pub depth: Option<usize>,
    /// Maximum nodes drawn before the graph is truncated.
    pub node_limit: Option<usize>,
}

/// The stdio MCP server. Holds the loaded index behind an `Arc` so the handler
/// stays cheap to clone, as the serve machinery requires.
#[derive(Clone)]
pub struct CallscopeServer {
    state: Arc<IndexState>,
    // Read at run time by the `#[tool_handler]`-generated dispatch code, which
    // the dead-code lint cannot see through the macro.
    #[allow(dead_code)]
    tool_router: ToolRouter<CallscopeServer>,
}

#[tool_router]
impl CallscopeServer {
    /// Wrap an already-loaded [`IndexState`] as an MCP server.
    pub fn new(state: Arc<IndexState>) -> Self {
        Self {
            state,
            tool_router: Self::tool_router(),
        }
    }

    // ── shared helpers ────────────────────────────────────────────────────────

    /// Run the per-request staleness check (Q6).
    fn stale(&self) -> Result<Option<StaleInfo>, ErrorData> {
        self.state
            .compute_stale()
            .map_err(|e| ErrorData::internal_error(format!("staleness check failed: {e}"), None))
    }

    /// Serialize an envelope (with staleness attached) to a tool text result.
    fn respond<T: serde::Serialize>(
        &self,
        env: Envelope<T>,
        stale: Option<StaleInfo>,
    ) -> Result<CallToolResult, ErrorData> {
        let json = envelope_to_json(env, stale)
            .map_err(|e| ErrorData::internal_error(format!("serialize failed: {e}"), None))?;
        Ok(CallToolResult::success(vec![ContentBlock::text(json)]))
    }

    /// Resolve a single symbol name and answer with `f`, or return the candidate
    /// set when the name is ambiguous or unknown (Q3). Staleness is attached to
    /// whichever envelope goes out.
    fn respond_resolved<T: serde::Serialize>(
        &self,
        name: &str,
        limit: usize,
        stale: Option<StaleInfo>,
        f: impl FnOnce(&IndexState, SymbolId) -> Envelope<T>,
    ) -> Result<CallToolResult, ErrorData> {
        match self.state.resolve_then(name, limit, |id| f(&self.state, id)) {
            Ok(env) => self.respond(env, stale),
            Err(candidates) => self.respond(candidates, stale),
        }
    }

    // ── C1 ──────────────────────────────────────────────────────────────────

    #[tool(
        name = "resolve_symbol",
        description = "C1: resolve a name or fragment to the candidate set of matching symbols with their characteristics. Never picks one — returns every match so an ambiguous name is visible."
    )]
    async fn resolve_symbol(
        &self,
        Parameters(args): Parameters<ResolveArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let stale = self.stale()?;
        let limit = args.limit.unwrap_or(DEFAULT_LIMIT);
        let env = self.state.resolve_symbol(&args.name, limit);
        self.respond(env, stale)
    }

    // ── C2 ──────────────────────────────────────────────────────────────────

    #[tool(
        name = "direct_calls",
        description = "C2: the direct callers and direct callees of a symbol (one hop each way)."
    )]
    async fn direct_calls(
        &self,
        Parameters(args): Parameters<SymbolArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let stale = self.stale()?;
        let limit = args.limit.unwrap_or(DEFAULT_LIMIT);
        self.respond_resolved(&args.symbol, limit, stale, |s, id| s.direct_calls(id, limit))
    }

    // ── C3 ──────────────────────────────────────────────────────────────────

    #[tool(
        name = "reachability",
        description = "C3: the transitive set reachable from a symbol in a direction (forward = callees, backward = callers)."
    )]
    async fn reachability(
        &self,
        Parameters(args): Parameters<ReachabilityArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let stale = self.stale()?;
        let limit = args.limit.unwrap_or(DEFAULT_LIMIT);
        let dir: Direction = args.direction.unwrap_or_default().into();
        self.respond_resolved(&args.symbol, limit, stale, |s, id| {
            s.reachability(id, dir, limit)
        })
    }

    // ── C4 ──────────────────────────────────────────────────────────────────

    #[tool(
        name = "call_paths",
        description = "C4: enumerate the simple call paths from one symbol to another, bounded by depth and count."
    )]
    async fn call_paths(
        &self,
        Parameters(args): Parameters<CallPathsArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let stale = self.stale()?;
        let max_depth = args.max_depth.unwrap_or(DEFAULT_MAX_DEPTH);
        let max_paths = args.max_paths.unwrap_or(DEFAULT_MAX_PATHS);
        match self
            .state
            .call_paths_by_name(&args.from, &args.to, max_depth, max_paths, DEFAULT_LIMIT)
        {
            Ok(env) => self.respond(env, stale),
            Err(candidates) => self.respond(candidates, stale),
        }
    }

    // ── C5 ──────────────────────────────────────────────────────────────────

    #[tool(
        name = "affected_tests",
        description = "C5: the tests that transitively reach a symbol — what to run after changing it. Includes tests reachable only through generic or dyn dispatch (the envelope flags the latter as over-approximated)."
    )]
    async fn affected_tests(
        &self,
        Parameters(args): Parameters<SymbolArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let stale = self.stale()?;
        let limit = args.limit.unwrap_or(DEFAULT_LIMIT);
        self.respond_resolved(&args.symbol, limit, stale, |s, id| s.affected_tests(id, limit))
    }

    // ── C6 ──────────────────────────────────────────────────────────────────

    #[tool(
        name = "reachable_unsafe",
        description = "C6: the unsafe-using symbols reachable forward from a symbol."
    )]
    async fn reachable_unsafe(
        &self,
        Parameters(args): Parameters<SymbolArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let stale = self.stale()?;
        let limit = args.limit.unwrap_or(DEFAULT_LIMIT);
        self.respond_resolved(&args.symbol, limit, stale, |s, id| {
            s.reachable_unsafe(id, limit)
        })
    }

    // ── C7 ──────────────────────────────────────────────────────────────────

    #[tool(
        name = "impact",
        description = "C7: the combined change impact of a symbol — its direct callers plus the tests that transitively reach it, in one answer."
    )]
    async fn impact(
        &self,
        Parameters(args): Parameters<SymbolArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let stale = self.stale()?;
        let limit = args.limit.unwrap_or(DEFAULT_LIMIT);
        self.respond_resolved(&args.symbol, limit, stale, |s, id| s.impact(id, limit))
    }

    // ── C8 ──────────────────────────────────────────────────────────────────

    #[tool(
        name = "neighborhood_graph",
        description = "C8: a Mermaid flowchart of the bounded call-graph neighborhood around a symbol. Dashed edges are dyn dispatch; thick edges leave the workspace."
    )]
    async fn neighborhood_graph(
        &self,
        Parameters(args): Parameters<NeighborhoodArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let stale = self.stale()?;
        let depth = args.depth.unwrap_or(DEFAULT_GRAPH_DEPTH);
        let node_limit = args.node_limit.unwrap_or(DEFAULT_NODE_LIMIT);
        self.respond_resolved(&args.symbol, DEFAULT_LIMIT, stale, |s, id| {
            s.neighborhood_graph(id, depth, node_limit)
        })
    }
}

#[tool_handler]
impl ServerHandler for CallscopeServer {
    fn get_info(&self) -> ServerInfo {
        // ServerInfo (InitializeResult) is #[non_exhaustive], so it cannot be
        // built with a struct literal from this crate. Start from its default —
        // which carries the current protocol version and the build-env server
        // identity — and set only what this server overrides.
        let mut info = ServerInfo::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.instructions = Some(INSTRUCTIONS.to_string());
        // Report this crate's identity rather than the SDK's build-env default,
        // which would otherwise name the server "rmcp". Implementation is also
        // #[non_exhaustive], so mutate fields on a default rather than build one.
        let mut me = Implementation::from_build_env();
        me.name = env!("CARGO_PKG_NAME").to_string();
        me.version = env!("CARGO_PKG_VERSION").to_string();
        info.server_info = me;
        info
    }
}
