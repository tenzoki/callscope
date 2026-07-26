//! `callscope-index` — the compiler-linked indexing engine (plan step 4 / P4).
//!
//! The ONLY crate in the workspace that links the Rust compiler. It runs a
//! target cargo workspace through monomorphization-grounded call-graph
//! extraction and writes `.callscope/index.bin` + `.callscope/manifest.json`.
//!
//! Two roles, one binary:
//!
//! - **Orchestrator** (`callscope-index <workspace-path>`): drives the target
//!   workspace's `cargo test --no-run` build with this binary set as
//!   `RUSTC_WRAPPER`, then merges the per-crate fragments into the index. Test
//!   mode seeds `#[test]` items as monomorphization roots (C5).
//! - **Wrapper** (invoked by cargo, flagged via `CALLSCOPE_WRAPPER`): compiles
//!   one crate and, for workspace members, extracts its call-graph fragment.
//!   See [`driver`].
//!
//! All shared types come from `callscope-core`; this crate defines none of the
//! schema itself.
#![feature(rustc_private)]

extern crate rustc_driver;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_public;
extern crate rustc_span;

mod driver;
mod graph_build;

use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, ExitCode};

use callscope_core::{fingerprint, Edge, EdgeKind, Index, Symbol, SymbolId, SCHEMA_VERSION};

use crate::graph_build::{DynCall, Fragment, ImplMethodFact};

/// Sysroot of the pinned toolchain, baked in by `build.rs`.
const SYSROOT: &str = env!("CALLSCOPE_SYSROOT");

fn main() -> ExitCode {
    if std::env::var_os("CALLSCOPE_WRAPPER").is_some() {
        return driver::wrapper_main();
    }

    let args: Vec<String> = std::env::args().collect();
    let Some(ws) = args.get(1) else {
        eprintln!("usage: callscope-index <workspace-path>");
        return ExitCode::FAILURE;
    };
    match orchestrate(Path::new(ws)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("callscope-index: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Build (or refresh) the index for the workspace rooted at `ws`.
fn orchestrate(ws: &Path) -> Result<(), String> {
    let ws = ws
        .canonicalize()
        .map_err(|e| format!("cannot resolve workspace path {}: {e}", ws.display()))?;
    if !ws.join("Cargo.toml").is_file() {
        return Err(format!("no Cargo.toml at {}", ws.display()));
    }

    let toolchain = toolchain_name();
    let self_exe =
        std::env::current_exe().map_err(|e| format!("cannot find own executable: {e}"))?;

    // Isolated scratch: a private target dir (so every crate is really compiled
    // through our wrapper, not served from an unrelated cache) and a fragment
    // drop directory.
    //
    // The whole scratch is wiped at the start of every run. This is a
    // correctness requirement, not just hygiene: cargo's incremental cache
    // would otherwise serve a workspace member from a previous build, so our
    // wrapper never re-runs for it and no fragment is emitted — yet we always
    // rebuild the index from *only* the fragments present, so a cached member
    // would silently vanish from the graph (a Q1 "silent wrong answer"). A
    // cold rebuild of every workspace member each run is the price of a
    // complete, deterministic index; deciding *whether* to re-index at all is
    // the staleness manifest's job (P3), upstream of this call.
    let scratch = ws.join(".callscope").join("build");
    let out_dir = scratch.join("fragments");
    let target_dir = scratch.join("target");
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&out_dir).map_err(|e| format!("cannot create {}: {e}", out_dir.display()))?;

    eprintln!(
        "callscope-index: indexing {} with toolchain {toolchain}",
        ws.display()
    );

    let lib_dir = format!("{SYSROOT}/lib");
    let status = Command::new("cargo")
        .current_dir(&ws)
        .args(["test", "--no-run"])
        .env("RUSTUP_TOOLCHAIN", &toolchain)
        .env("RUSTC_WRAPPER", &self_exe)
        .env("CALLSCOPE_WRAPPER", "1")
        .env("CALLSCOPE_OUT_DIR", &out_dir)
        .env("CARGO_TARGET_DIR", &target_dir)
        // macOS/Linux: let the wrapper subprocess load the compiler dylib.
        .env("DYLD_FALLBACK_LIBRARY_PATH", &lib_dir)
        .env("LD_LIBRARY_PATH", &lib_dir)
        .status()
        .map_err(|e| format!("failed to run cargo: {e}"))?;
    if !status.success() {
        return Err("target workspace build failed under the indexer".to_string());
    }

    let fragments = read_fragments(&out_dir)?;
    let (symbols, edges) = merge(fragments)?;

    let index = Index {
        schema_version: SCHEMA_VERSION,
        symbols,
        edges,
    };
    let manifest = fingerprint::fingerprint_workspace(&ws, &toolchain)
        .map_err(|e| format!("fingerprint failed: {e}"))?;

    let dot = ws.join(".callscope");
    std::fs::create_dir_all(&dot).map_err(|e| format!("cannot create {}: {e}", dot.display()))?;
    let index_path = dot.join("index.bin");
    let manifest_path = dot.join("manifest.json");
    std::fs::write(
        &index_path,
        serde_json::to_vec(&index).map_err(|e| format!("serialize index: {e}"))?,
    )
    .map_err(|e| format!("write {}: {e}", index_path.display()))?;
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).map_err(|e| format!("serialize manifest: {e}"))?,
    )
    .map_err(|e| format!("write {}: {e}", manifest_path.display()))?;

    eprintln!(
        "callscope-index: wrote {} ({} symbols, {} edges) and {}",
        index_path.display(),
        index.symbols.len(),
        index.edges.len(),
        manifest_path.display(),
    );
    Ok(())
}

/// The rustup toolchain name is the basename of the baked-in sysroot, e.g.
/// `nightly-2026-07-26-aarch64-apple-darwin`.
fn toolchain_name() -> String {
    Path::new(SYSROOT)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown".to_string())
}

fn read_fragments(dir: &Path) -> Result<Vec<Fragment>, String> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|e| format!("read {}: {e}", dir.display()))? {
        let entry = entry.map_err(|e| format!("read dir entry: {e}"))?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let bytes = std::fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let frag: Fragment = serde_json::from_slice(&bytes)
            .map_err(|e| format!("parse fragment {}: {e}", path.display()))?;
        out.push(frag);
    }
    Ok(out)
}

/// Merge per-crate fragments into one deduplicated symbol/edge set.
///
/// **fail-on-collision:** the schema promises that two distinct fully-qualified
/// paths hashing to the same [`SymbolId`] are detected by the indexer rather
/// than silently merged (which `Graph::new`'s last-write-wins would otherwise
/// do). We enforce that here: a collision aborts the index build with a
/// diagnostic naming both paths.
fn merge(fragments: Vec<Fragment>) -> Result<(Vec<Symbol>, Vec<Edge>), String> {
    let mut symbols: HashMap<SymbolId, Symbol> = HashMap::new();
    let mut collisions: Vec<(u64, String, String)> = Vec::new();
    let mut edge_seen: std::collections::HashSet<(u64, u64, u8)> = std::collections::HashSet::new();
    let mut edges: Vec<Edge> = Vec::new();
    // Collected across all fragments for the workspace-wide `dyn` join below.
    let mut dyn_calls: Vec<DynCall> = Vec::new();
    let mut impl_methods: Vec<ImplMethodFact> = Vec::new();

    for frag in fragments {
        dyn_calls.extend(frag.dyn_calls);
        impl_methods.extend(frag.impl_methods);
        for sym in frag.symbols {
            match symbols.get(&sym.id) {
                Some(existing) if existing.fq_path != sym.fq_path => {
                    collisions.push((sym.id.0, existing.fq_path.clone(), sym.fq_path.clone()));
                }
                Some(existing) => {
                    // Same symbol seen in another fragment. Merge characteristics
                    // monotonically: a fact observed in any compilation holds
                    // (e.g. `test` is only visible in the `--test` compilation,
                    // `uses_unsafe` only where the body was walked).
                    let mut merged = existing.clone();
                    let c = &mut merged.characteristics;
                    let n = &sym.characteristics;
                    c.test |= n.test;
                    c.public |= n.public;
                    c.is_async |= n.is_async;
                    c.generic |= n.generic;
                    c.foreign |= n.foreign;
                    c.uses_unsafe |= n.uses_unsafe;
                    symbols.insert(sym.id, merged);
                }
                None => {
                    symbols.insert(sym.id, sym);
                }
            }
        }
        for edge in frag.edges {
            let disc = match edge.kind {
                callscope_core::EdgeKind::Static => 0u8,
                callscope_core::EdgeKind::Virtual => 1u8,
            };
            if edge_seen.insert((edge.from.0, edge.to.0, disc)) {
                edges.push(edge);
            }
        }
    }

    if !collisions.is_empty() {
        let mut msg = String::from(
            "SymbolId collision detected: distinct fully-qualified paths hashed to the same id. \
             Refusing to write an index that would silently merge them.\n",
        );
        for (id, a, b) in &collisions {
            msg.push_str(&format!("  id {id:#018x}: {a:?} vs {b:?}\n"));
        }
        return Err(msg);
    }

    // ── `dyn` over-approximation: workspace-wide implementor join (gap 2) ──
    //
    // Each `dyn`/unresolved trait-method call site (`DynCall`) is widened to
    // every implementor of that `(trait, method)` across the WHOLE workspace —
    // not just the call site's own crate. This is what closes the two
    // real-workspace gaps: implementors in a different member crate are visible
    // because every crate's `ImplMethodFact`s are collected here, and generic
    // implementors are included because they were inventoried by their
    // polymorphic method name (walked once, un-monomorphized, in the defining
    // crate). Every synthesised edge is `Virtual`, so the query layer flags the
    // answer over-approximated and counts these implementors honestly.
    //
    // The count stays honest by construction: one `Virtual` edge per distinct
    // (owner, implementor-method), and `query.rs` derives `implementor_count`
    // from the distinct virtual-edge targets a walk folds in. No exact claim is
    // made — a generic implementor's edge points at its generic method symbol
    // (flagged `generic`), which is a superset representative, never a false
    // monomorphized certainty.
    let mut impls_by_key: HashMap<(&str, &str), Vec<&ImplMethodFact>> = HashMap::new();
    for fact in &impl_methods {
        impls_by_key
            .entry((fact.trait_path.as_str(), fact.method.as_str()))
            .or_default()
            .push(fact);
    }
    for call in &dyn_calls {
        let Some(facts) = impls_by_key.get(&(call.trait_path.as_str(), call.method.as_str()))
        else {
            continue;
        };
        for fact in facts {
            let to = SymbolId::from_fq_path(&fact.fq_path);
            if edge_seen.insert((call.owner.0, to.0, 1u8)) {
                edges.push(Edge {
                    from: call.owner,
                    to,
                    kind: EdgeKind::Virtual,
                });
            }
        }
    }

    // Keep only edges that originate from a known workspace symbol. A missing
    // `from` cannot happen (we only emit edges from walked symbols), but this
    // keeps the invariant explicit; the `to` end is intentionally left dangling
    // when it crosses the workspace boundary, which the query layer reads as a
    // boundary crossing.
    edges.retain(|e| symbols.contains_key(&e.from));

    // Sort edges for a byte-deterministic index. The edge SET is already fixed
    // by the cold rebuild, but its order otherwise follows fragment read order
    // (filesystem-dependent) and the merge-time join's hash-map iteration, so
    // without this the artifact's bytes vary run to run. Query results do not
    // depend on this (the query layer re-sorts adjacency), but a stable artifact
    // is what the orchestrator's "deterministic index" contract promises.
    let disc = |k: callscope_core::EdgeKind| match k {
        callscope_core::EdgeKind::Static => 0u8,
        callscope_core::EdgeKind::Virtual => 1u8,
    };
    edges.sort_by(|a, b| {
        (a.from.0, a.to.0, disc(a.kind)).cmp(&(b.from.0, b.to.0, disc(b.kind)))
    });

    let mut symbols: Vec<Symbol> = symbols.into_values().collect();
    symbols.sort_by(|a, b| (a.fq_path.as_str(), a.id.0).cmp(&(b.fq_path.as_str(), b.id.0)));
    Ok((symbols, edges))
}

#[cfg(test)]
mod tests {
    use super::*;
    use callscope_core::{Characteristics, Span};
    use crate::graph_build::{DynCall, Fragment, ImplMethodFact};

    fn sym(id: u64, fq: &str) -> Symbol {
        Symbol {
            id: SymbolId(id),
            fq_path: fq.to_string(),
            crate_name: "c".to_string(),
            span: Span { file: "src/lib.rs".to_string(), line_start: 1, line_end: 1 },
            characteristics: Characteristics {
                test: false, public: true, is_async: false,
                generic: false, foreign: false, uses_unsafe: false,
            },
        }
    }

    /// fail-on-collision: two distinct `fq_path`s carrying the same `SymbolId`
    /// (whether from one crate or two) must abort the merge with a diagnostic
    /// naming both paths, never silently merge.
    #[test]
    fn merge_fails_on_symbolid_collision() {
        let colliding_id = 0x1234_5678_9abc_def0;
        let frag_a = Fragment {
            symbols: vec![sym(colliding_id, "crate_a::alpha")],
            edges: vec![],
            ..Default::default()
        };
        let frag_b = Fragment {
            symbols: vec![sym(colliding_id, "crate_b::beta")],
            edges: vec![],
            ..Default::default()
        };
        let err = merge(vec![frag_a, frag_b]).expect_err("collision must fail the build");
        assert!(err.contains("collision"), "diagnostic must mention collision: {err}");
        assert!(err.contains("crate_a::alpha") && err.contains("crate_b::beta"),
            "diagnostic must name both colliding paths: {err}");
    }

    /// The same symbol seen in two fragments (identical id AND path) is a normal
    /// merge, not a collision, and its characteristics union monotonically.
    #[test]
    fn merge_unions_same_symbol_across_fragments() {
        let id = 0xabc;
        let mut a = sym(id, "crate::f");
        a.characteristics.test = true; // visible only in the --test compilation
        let b = sym(id, "crate::f"); // uses_unsafe/public seen elsewhere
        let (symbols, _edges) =
            merge(vec![Fragment { symbols: vec![a], edges: vec![], ..Default::default() },
                      Fragment { symbols: vec![b], edges: vec![], ..Default::default() }])
                .expect("same symbol twice is not a collision");
        assert_eq!(symbols.len(), 1);
        assert!(symbols[0].characteristics.test, "characteristics must union");
    }

    /// The `dyn` over-approximation join is workspace-wide (gap 2): a `DynCall`
    /// in one crate's fragment must widen to implementors inventoried in *other*
    /// crates' fragments, and to a *generic* implementor, each as a `Virtual`
    /// edge. This is the merge-time half of the two closed dyn-dispatch gaps.
    #[test]
    fn merge_joins_dyn_calls_to_all_workspace_implementors() {
        // Fragment A (crate "app"): the `&dyn Tokenizer` call site in `run_dyn`,
        // plus one same-crate non-generic implementor and one generic one.
        // Symbols must carry the same content-addressed id the join derives from
        // their fq_path (that is how a `DynCall.owner` matches its symbol).
        let symf = |fq: &str| {
            let mut s = sym(0, fq);
            s.id = SymbolId::from_fq_path(fq);
            s
        };
        let frag_app = Fragment {
            symbols: vec![
                symf("app::run_dyn"),
                symf("<app::Simple as app::Tokenizer>::tokenize"),
                symf("<app::Wrapper<T> as app::Tokenizer>::tokenize"),
            ],
            edges: vec![],
            dyn_calls: vec![DynCall {
                owner: SymbolId::from_fq_path("app::run_dyn"),
                trait_path: "app::Tokenizer".to_string(),
                method: "tokenize".to_string(),
            }],
            impl_methods: vec![
                ImplMethodFact {
                    trait_path: "app::Tokenizer".to_string(),
                    method: "tokenize".to_string(),
                    fq_path: "<app::Simple as app::Tokenizer>::tokenize".to_string(),
                    generic: false,
                },
                ImplMethodFact {
                    trait_path: "app::Tokenizer".to_string(),
                    method: "tokenize".to_string(),
                    fq_path: "<app::Wrapper<T> as app::Tokenizer>::tokenize".to_string(),
                    generic: true,
                },
            ],
        };
        // Fragment B (a DIFFERENT member crate "ext"): a cross-crate implementor
        // of `app`'s trait. It carries no `DynCall`; only its inventory.
        let frag_ext = Fragment {
            symbols: vec![symf("<ext::Shouty as app::Tokenizer>::tokenize")],
            edges: vec![],
            dyn_calls: vec![],
            impl_methods: vec![ImplMethodFact {
                trait_path: "app::Tokenizer".to_string(),
                method: "tokenize".to_string(),
                fq_path: "<ext::Shouty as app::Tokenizer>::tokenize".to_string(),
                generic: false,
            }],
        };

        let (_symbols, edges) =
            merge(vec![frag_app, frag_ext]).expect("no collisions");

        let owner = SymbolId::from_fq_path("app::run_dyn");
        let virtual_targets: std::collections::HashSet<SymbolId> = edges
            .iter()
            .filter(|e| e.from == owner && e.kind == EdgeKind::Virtual)
            .map(|e| e.to)
            .collect();

        // All three implementors — same-crate, generic, AND cross-crate — must
        // be widened to, each via a Virtual edge.
        for fq in [
            "<app::Simple as app::Tokenizer>::tokenize",
            "<app::Wrapper<T> as app::Tokenizer>::tokenize",
            "<ext::Shouty as app::Tokenizer>::tokenize",
        ] {
            assert!(
                virtual_targets.contains(&SymbolId::from_fq_path(fq)),
                "dyn over-approximation must include implementor {fq}",
            );
        }
        assert_eq!(
            virtual_targets.len(),
            3,
            "exactly the three workspace implementors, no more, no fewer",
        );
    }
}
