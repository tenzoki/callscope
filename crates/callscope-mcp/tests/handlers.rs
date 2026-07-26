//! Integration test for the callscope-mcp answer path.
//!
//! No MCP client is involved (and none is needed): the test builds a small
//! `Index` by hand, serializes it to a temp workspace exactly as the server
//! reads it, loads an `IndexState`, and calls the handler methods directly —
//! asserting the tools return well-formed envelopes and that the staleness check
//! (Q6) flips when a source file diverges from the manifest.
//!
//! This deliberately does NOT depend on a real `.callscope/index.bin` produced
//! by `callscope-index` (P4), which is being built in parallel and may not exist.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use callscope_core::query::Direction;
use callscope_core::{
    Characteristics, Edge, EdgeKind, Index, Manifest, Span, Symbol, SymbolId, SCHEMA_VERSION,
};

// The module under test is a binary crate; pull in its two source modules
// directly so the handler logic is exercised without a `lib.rs`.
#[path = "../src/state.rs"]
mod state;

use state::{IndexState, DEFAULT_LIMIT};

// ── fixture builders ─────────────────────────────────────────────────────────

fn chars() -> Characteristics {
    Characteristics {
        test: false,
        public: true,
        is_async: false,
        generic: false,
        foreign: false,
        uses_unsafe: false,
    }
}

fn sym_with(fq: &str, mutate: impl FnOnce(&mut Characteristics)) -> Symbol {
    let mut c = chars();
    mutate(&mut c);
    Symbol {
        id: SymbolId::from_fq_path(fq),
        fq_path: fq.to_string(),
        crate_name: fq.split("::").next().unwrap_or("").to_string(),
        span: Span {
            file: "src/lib.rs".to_string(),
            line_start: 1,
            line_end: 2,
        },
        characteristics: c,
    }
}

fn sym(fq: &str) -> Symbol {
    sym_with(fq, |_| {})
}

fn edge(from: &str, to: &str, kind: EdgeKind) -> Edge {
    Edge {
        from: SymbolId::from_fq_path(from),
        to: SymbolId::from_fq_path(to),
        kind,
    }
}

/// A miniature of the guiding example: a test reaches `normalize_token` only
/// through a dyn-dispatch (virtual) edge, so C5 must find it AND flag Q2.
fn sample_index() -> Index {
    Index {
        schema_version: SCHEMA_VERSION,
        symbols: vec![
            sym("parser::normalize_token"),
            sym("parser::normalize_line"),
            sym("parser::run_generic"),
            sym_with("parser::tests::dyn_test", |c| c.test = true),
            sym_with("parser::danger", |c| c.uses_unsafe = true),
        ],
        edges: vec![
            edge("parser::tests::dyn_test", "parser::run_generic", EdgeKind::Static),
            edge("parser::run_generic", "parser::normalize_token", EdgeKind::Virtual),
            edge("parser::normalize_token", "parser::danger", EdgeKind::Static),
        ],
    }
}

/// Write a workspace to a fresh temp dir: a `.callscope/` with the serialized
/// index + a manifest whose `file_hashes` match the `.rs` files actually on
/// disk, so a clean load reports no staleness.
fn write_workspace(index: &Index) -> PathBuf {
    // Unique per invocation so tests running in parallel never share a dir.
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "callscope-mcp-test-{}-{}",
        std::process::id(),
        n
    ));
    // Fresh each run.
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join(".callscope")).unwrap();

    // One source file + a Cargo.lock, so the fingerprint has something to hash.
    fs::write(root.join("src/lib.rs"), "pub fn normalize_token() {}\n").unwrap();
    fs::write(root.join("Cargo.lock"), "# lock\n").unwrap();

    // Serialize the index as the server reads it (JSON).
    fs::write(
        root.join(".callscope/index.bin"),
        serde_json::to_vec(index).unwrap(),
    )
    .unwrap();

    // Build a manifest whose hashes match the files just written, via the same
    // fingerprint code the server uses — so a fresh load is NOT stale.
    let manifest =
        callscope_core::fingerprint::fingerprint_workspace(&root, "test-toolchain").unwrap();
    fs::write(
        root.join(".callscope/manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();

    root
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[test]
fn loads_from_disk_and_round_trips_the_index() {
    let idx = sample_index();
    let root = write_workspace(&idx);
    let state = IndexState::load(&root).expect("load");

    // C1: resolving a fragment returns the two normalize_* candidates, sorted.
    let env = state.resolve_symbol("normalize", DEFAULT_LIMIT);
    let names: Vec<String> = env.data.iter().map(|s| s.fq_path.clone()).collect();
    assert_eq!(
        names,
        vec!["parser::normalize_line", "parser::normalize_token"]
    );
    assert_eq!(env.total, 2);
    assert!(!env.truncated);
}

#[test]
fn affected_tests_crosses_dyn_dispatch_and_flags_over_approximation() {
    let idx = sample_index();
    let root = write_workspace(&idx);
    let state = IndexState::load(&root).expect("load");

    let target = SymbolId::from_fq_path("parser::normalize_token");
    let env = state.affected_tests(target, DEFAULT_LIMIT);

    // The test reaches the target only through the virtual edge — it must appear.
    let names: Vec<String> = env.data.iter().map(|s| s.fq_path.clone()).collect();
    assert_eq!(names, vec!["parser::tests::dyn_test"]);
    // ...and the answer must be flagged over-approximated (Q2).
    assert!(
        env.over_approximated.is_some(),
        "reaching through dyn dispatch must over-approximate"
    );
}

#[test]
fn impact_and_direct_calls_serialize_to_well_formed_envelopes() {
    let idx = sample_index();
    let root = write_workspace(&idx);
    let state = IndexState::load(&root).expect("load");

    let target = SymbolId::from_fq_path("parser::normalize_token");

    // C7 impact — payload was remapped to a serializable mirror; check the JSON
    // has the mirror's fields and the always-present envelope flags.
    let impact = state.impact(target, DEFAULT_LIMIT);
    let json = state::envelope_to_json(impact, None).expect("serialize impact");
    assert!(json.contains("direct_callers"), "{json}");
    assert!(json.contains("affected_tests"), "{json}");
    assert!(json.contains("\"truncated\":false"), "{json}");
    assert!(json.contains("\"boundary_applies\":false"), "{json}");

    // C2 direct_calls — run_generic calls normalize_token (virtually).
    let dc = state.direct_calls(target, DEFAULT_LIMIT);
    assert_eq!(
        dc.data
            .callers
            .iter()
            .map(|s| s.fq_path.clone())
            .collect::<Vec<_>>(),
        vec!["parser::run_generic"]
    );
    assert_eq!(
        dc.data
            .callees
            .iter()
            .map(|s| s.fq_path.clone())
            .collect::<Vec<_>>(),
        vec!["parser::danger"]
    );
    let json = state::envelope_to_json(dc, None).expect("serialize direct_calls");
    assert!(json.contains("callers") && json.contains("callees"), "{json}");
}

#[test]
fn reachable_unsafe_and_neighborhood_and_reachability() {
    let idx = sample_index();
    let root = write_workspace(&idx);
    let state = IndexState::load(&root).expect("load");

    let target = SymbolId::from_fq_path("parser::normalize_token");

    // C6: danger is reachable-unsafe from normalize_token.
    let unsafe_env = state.reachable_unsafe(target, DEFAULT_LIMIT);
    assert_eq!(
        unsafe_env
            .data
            .iter()
            .map(|s| s.fq_path.clone())
            .collect::<Vec<_>>(),
        vec!["parser::danger"]
    );

    // C3: backward reachability from normalize_token reaches run_generic + test.
    let back = state.reachability(target, Direction::Backward, DEFAULT_LIMIT);
    assert_eq!(back.total, 2);

    // C8: neighborhood graph renders Mermaid flowchart text.
    let graph = state.neighborhood_graph(target, 2, 60);
    assert!(
        graph.data.starts_with("flowchart"),
        "expected mermaid flowchart, got: {}",
        graph.data
    );
}

#[test]
fn staleness_flips_when_a_source_file_diverges() {
    let idx = sample_index();
    let root = write_workspace(&idx);
    let state = IndexState::load(&root).expect("load");

    // Freshly loaded: manifest matches disk, so not stale (Q6).
    assert!(state.compute_stale().expect("stale check").is_none());

    // Edit a source file — the fingerprint must now report it diverged.
    fs::write(root.join("src/lib.rs"), "pub fn normalize_token() { /* edited */ }\n").unwrap();
    let stale = state.compute_stale().expect("stale check");
    let info = stale.expect("index must be stale after an edit");
    assert!(
        info.diverged_files.iter().any(|f| f.contains("lib.rs")),
        "diverged files should name lib.rs: {:?}",
        info.diverged_files
    );
}
