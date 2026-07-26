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

// Re-export the schema and envelope types at the crate root: they are the
// vocabulary every other crate speaks, so `callscope_core::Envelope` reads
// better than `callscope_core::envelope::Envelope`.
pub use envelope::{Envelope, Reason, StaleInfo};
pub use schema::{
    Characteristics, Edge, EdgeKind, Index, Manifest, Span, Symbol, SymbolId, SCHEMA_VERSION,
};
