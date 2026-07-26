//! Index schema — the serde-serializable call graph that `callscope-index`
//! writes to disk and `callscope-mcp` reads back.
//!
//! The graph is two flat vectors: [`Symbol`]s (functions and methods the
//! indexer resolved) and directed call [`Edge`]s between them. Edges reference
//! symbols by [`SymbolId`] rather than by position, so the two vectors can be
//! reordered, filtered, or grown across re-indexing runs without invalidating
//! the references.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// On-disk schema version. Bump when a change to any type in this module makes
/// an older `index.bin` / `manifest.json` unreadable. The reader compares this
/// against the value stored in a loaded [`Index`] / [`Manifest`] to reject a
/// mismatched artifact rather than misparse it.
pub const SCHEMA_VERSION: u32 = 1;

/// Stable identifier for a [`Symbol`].
///
/// **Representation: a 64-bit FNV-1a hash of the fully-qualified path**, not a
/// positional index into [`Index::symbols`]. The requirement is stability
/// across re-indexing of unchanged code — a caller who resolved a symbol in one
/// run must be able to reference it in the next. A positional `u32` index fails
/// that the moment any symbol is added, removed, or reordered, because every
/// later index shifts. Content-addressing by `fq_path` keeps the id fixed as
/// long as the path is unchanged.
///
/// FNV-1a is chosen over the standard-library hasher (SipHash) because SipHash's
/// output is not guaranteed stable across Rust versions or platforms, which
/// would let an id drift under a toolchain bump even though the code did not
/// change. FNV-1a is a fixed, dependency-free algorithm: the same input yields
/// the same id everywhere, forever. `u64` (rather than a hex string) keeps the
/// id `Copy` and compact, so it is cheap as an edge endpoint and a map key.
///
/// Collision risk is negligible at workspace scale: for a few thousand symbols
/// the 64-bit space makes a collision astronomically unlikely, and a workspace
/// large enough to matter would be caught by the indexer's own symbol table
/// (two distinct `fq_path`s hashing equal is detectable there).
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SymbolId(pub u64);

impl SymbolId {
    /// Derive the stable id from a fully-qualified path via FNV-1a (64-bit).
    pub fn from_fq_path(fq_path: &str) -> Self {
        const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
        const PRIME: u64 = 0x0000_0100_0000_01b3;
        let mut hash = OFFSET_BASIS;
        for byte in fq_path.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(PRIME);
        }
        SymbolId(hash)
    }
}

/// Source location of a symbol. Line numbers are 1-based, matching rustc spans.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Span {
    /// Path to the source file, workspace-relative.
    pub file: String,
    pub line_start: u32,
    pub line_end: u32,
}

/// The attributes `callscope-index` tags on each symbol from HIR and the
/// unsafety check (gap 4 in the plan). Every field is a plain boolean fact
/// about the function itself, not about anything it reaches.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Characteristics {
    /// Carries `#[test]` (or a test-framework attribute such as `#[tokio::test]`).
    pub test: bool,
    /// Visible outside its defining module (`pub` in effective visibility).
    pub public: bool,
    /// Declared `async`.
    pub is_async: bool,
    /// Declared with generic parameters.
    pub generic: bool,
    /// `extern`/foreign function (FFI).
    pub foreign: bool,
    /// Contains an `unsafe` block or is an `unsafe fn`.
    pub uses_unsafe: bool,
}

/// A function or method the indexer resolved: its stable id, fully-qualified
/// path, owning crate, source span, and characteristics (C1 in the plan).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub id: SymbolId,
    /// Fully-qualified path, e.g. `parser::normalize_token`. The id is derived
    /// from this string via [`SymbolId::from_fq_path`].
    pub fq_path: String,
    pub crate_name: String,
    pub span: Span,
    pub characteristics: Characteristics,
}

/// How a call edge was resolved.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    /// A statically resolved call — the concrete callee is known exactly.
    Static,
    /// A `dyn`-dispatch call, over-approximated to every workspace implementor
    /// of the trait. This is the origin of over-approximation (Q2): an answer
    /// that crosses a `Virtual` edge cannot claim the callee is exact.
    Virtual,
}

/// A directed call edge: `from` calls `to`, resolved as `kind`.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    pub from: SymbolId,
    pub to: SymbolId,
    pub kind: EdgeKind,
}

/// The whole resolved call graph, serialized to `index.bin`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Index {
    pub schema_version: u32,
    pub symbols: Vec<Symbol>,
    pub edges: Vec<Edge>,
}

impl Index {
    /// An empty index stamped with the current [`SCHEMA_VERSION`].
    pub fn empty() -> Self {
        Index {
            schema_version: SCHEMA_VERSION,
            symbols: Vec::new(),
            edges: Vec::new(),
        }
    }
}

/// The staleness manifest, serialized to `manifest.json` (Q5/Q6).
///
/// `file_hashes` is a [`BTreeMap`] rather than a `HashMap` so the JSON key order
/// is deterministic: the same workspace state serializes byte-for-byte the same,
/// which the fingerprint check (P3) relies on to compare cheaply and which keeps
/// the artifact diff-friendly.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub schema_version: u32,
    /// The exact toolchain used to build the index (records the nightly pin, so
    /// a later run can tell whether the toolchain itself changed).
    pub toolchain: String,
    /// Per-file content hash of every indexed `.rs` file, keyed by path.
    pub file_hashes: BTreeMap<String, String>,
    pub cargo_lock_hash: String,
    /// When the index was built (RFC-3339 UTC).
    pub indexed_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_symbol(fq: &str, crate_name: &str) -> Symbol {
        Symbol {
            id: SymbolId::from_fq_path(fq),
            fq_path: fq.to_string(),
            crate_name: crate_name.to_string(),
            span: Span {
                file: "src/lib.rs".to_string(),
                line_start: 10,
                line_end: 14,
            },
            characteristics: Characteristics {
                test: false,
                public: true,
                is_async: false,
                generic: false,
                foreign: false,
                uses_unsafe: false,
            },
        }
    }

    #[test]
    fn symbol_id_is_stable_for_same_path() {
        assert_eq!(
            SymbolId::from_fq_path("parser::normalize_token"),
            SymbolId::from_fq_path("parser::normalize_token"),
        );
    }

    #[test]
    fn symbol_id_differs_for_different_paths() {
        assert_ne!(
            SymbolId::from_fq_path("parser::normalize_token"),
            SymbolId::from_fq_path("parser::denormalize_token"),
        );
    }

    #[test]
    fn index_round_trips_through_serde() {
        let a = sample_symbol("parser::normalize_token", "parser");
        let b = sample_symbol("parser::run_generic", "parser");
        let index = Index {
            schema_version: SCHEMA_VERSION,
            edges: vec![Edge {
                from: b.id,
                to: a.id,
                kind: EdgeKind::Virtual,
            }],
            symbols: vec![a, b],
        };

        let bytes = serde_json::to_vec(&index).expect("serialize");
        let round: Index = serde_json::from_slice(&bytes).expect("deserialize");
        assert_eq!(index, round);
    }

    #[test]
    fn manifest_round_trips_and_keys_stay_sorted() {
        let mut file_hashes = BTreeMap::new();
        file_hashes.insert("src/zeta.rs".to_string(), "hashz".to_string());
        file_hashes.insert("src/alpha.rs".to_string(), "hasha".to_string());
        let manifest = Manifest {
            schema_version: SCHEMA_VERSION,
            toolchain: "nightly-2026-07-26".to_string(),
            file_hashes,
            cargo_lock_hash: "lockhash".to_string(),
            indexed_at: "2026-07-26T18:38:00Z".to_string(),
        };

        let json = serde_json::to_string(&manifest).expect("serialize");
        // BTreeMap guarantees sorted keys: alpha precedes zeta in the output.
        assert!(
            json.find("src/alpha.rs").unwrap() < json.find("src/zeta.rs").unwrap(),
            "file_hashes keys must serialize in sorted order for determinism",
        );

        let round: Manifest = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(manifest, round);
    }
}
