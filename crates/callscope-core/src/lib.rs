//! callscope-core — the stable shared foundation of the callscope workspace.
//!
//! Owns the index schema, the staleness fingerprint, the graph query
//! algorithms, and the single output envelope every MCP tool returns. Links no
//! compiler internals, so it builds on stable Rust.
//!
//! The modules below are declared here and filled by later plan tasks. Each is
//! an empty stub for now so the P1 scaffold builds green.

pub mod schema;
pub mod envelope;
pub mod fingerprint;
pub mod query;
pub mod mermaid;
