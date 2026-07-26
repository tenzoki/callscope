//! Acceptance harness (plan step P11) — the formal proof the Directive is met.
//!
//! Runs the **real** `callscope-index` binary against `fixtures/workspace/`,
//! loads the produced `index.bin` through the authoritative answer path
//! (`callscope-mcp`'s [`IndexState`], the same handlers the MCP tools call), and
//! asserts every capability C1–C8 and every quality requirement Q1–Q6 from
//! `problem.md` §3/§5/§6 is demonstrable against the fixture.
//!
//! # How to run
//!
//! The indexer must be built first (it is the only crate needing the pinned
//! nightly `rustc-dev`; the whole workspace is pinned via `rust-toolchain.toml`,
//! so one toolchain serves both):
//!
//! ```text
//! cargo build -p callscope-index
//! cargo test  -p callscope-mcp --test acceptance -- --nocapture
//! ```
//!
//! `--nocapture` surfaces the per-capability PASS/FAIL breakdown this harness
//! prints. If the `callscope-index` binary is not found, the harness fails with
//! the build command above (it deliberately does NOT shell out to a nested
//! `cargo build`, which would deadlock on the workspace's own target lock).
//! `CALLSCOPE_INDEX_BIN` overrides the binary location.
//!
//! # Why it shells out to the real indexer
//!
//! `.callscope/` is gitignored (see the P4 coder's note), so there is no
//! committed index to load. The harness must produce ground truth itself by
//! running the compiler-linked indexer, which is exactly what makes this an
//! end-to-end acceptance test rather than a unit test over a hand-built graph.

use std::collections::BTreeSet;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::Command;

use callscope_core::query::Direction;
use callscope_core::{Symbol, SymbolId};

// The answer path is a binary crate with no `lib.rs`; pull its `state` module in
// directly, the same way `tests/handlers.rs` does. `state.rs` links no MCP wire
// code (that lives in `tools.rs`), so it compiles standalone under the test's
// dependency set.
#[path = "../src/state.rs"]
mod state;

use state::{IndexState, DEFAULT_GRAPH_DEPTH, DEFAULT_LIMIT, DEFAULT_MAX_DEPTH, DEFAULT_MAX_PATHS};

// ── the fixture's ground truth (see the P4 + FIX-DYN history notes) ────────────

const NORMALIZE: &str = "parser::normalize_token";
const RUN_GENERIC: &str = "parser::run_generic::<parser::Simple>";
const RUN_DYN: &str = "parser::run_dyn";
const ENSURE_ROUND_TRIP: &str = "parser::ensure_round_trip";

/// The four `Tokenizer` implementors the `run_dyn` over-approximation must widen
/// to (FIX-DYN): same-crate non-generic (Simple, Fancy), generic (`Wrapper<T>`),
/// and cross-crate (`ext_tokenizer::Shouty`).
const FOUR_IMPLEMENTORS: [&str; 4] = [
    "<parser::Simple as parser::Tokenizer>::tokenize",
    "<parser::Fancy as parser::Tokenizer>::tokenize",
    "<parser::Wrapper<T> as parser::Tokenizer>::tokenize",
    "<ext_tokenizer::Shouty as parser::Tokenizer>::tokenize",
];

/// Every test that transitively reaches `normalize_token`. The two load-bearing
/// §6 answers are the generic-dispatch cases (in-crate and separate-crate).
const EIGHT_AFFECTED_TESTS: [&str; 8] = [
    "parser::tests::normalizes_directly",
    "parser::tests::reaches_via_generic_dispatch", // load-bearing (in-crate, generic dispatch)
    "parser::tests::reaches_via_dyn_dispatch",
    "parser::tests::reaches_via_dyn_wrapper",
    "parser::tests::async_reaches_target", // #[tokio::test], reaches via async body
    "integration::integration_reaches_via_generic", // load-bearing (separate crate, generic dispatch)
    "integration::integration_reaches_via_dyn",
    "ext_tokenizer::tests::cross_crate_reaches_via_dyn",
];

// ── report framework ───────────────────────────────────────────────────────────

struct Row {
    id: String,
    desc: String,
    pass: bool,
    detail: String,
}

struct Report {
    rows: Vec<Row>,
}

impl Report {
    fn new() -> Self {
        Report { rows: Vec::new() }
    }

    /// Run one check. `f` returns `Ok(evidence)` on success or `Err(reason)` on
    /// a demonstrable failure; a panic inside `f` is caught and recorded as a
    /// failure so one broken check never hides the rest of the breakdown.
    fn check(&mut self, id: &str, desc: &str, f: impl FnOnce() -> Result<String, String>) {
        let (pass, detail) = match catch_unwind(AssertUnwindSafe(f)) {
            Ok(Ok(evidence)) => (true, evidence),
            Ok(Err(reason)) => (false, reason),
            Err(p) => (false, format!("PANIC: {}", panic_message(&p))),
        };
        self.rows.push(Row {
            id: id.to_string(),
            desc: desc.to_string(),
            pass,
            detail,
        });
    }

    /// Print the breakdown and fail the test if any check failed.
    fn finish(self) {
        let passed = self.rows.iter().filter(|r| r.pass).count();
        let total = self.rows.len();
        println!("\n════════════════════════════════════════════════════════════════════");
        println!(" callscope acceptance — C1–C8 and Q1–Q6 against fixtures/workspace/");
        println!("════════════════════════════════════════════════════════════════════");
        for r in &self.rows {
            let mark = if r.pass { "PASS" } else { "FAIL" };
            println!("[{mark}] {:<8} {}", r.id, r.desc);
            println!("          → {}", r.detail);
        }
        println!("────────────────────────────────────────────────────────────────────");
        println!(" {passed}/{total} checks passed");
        println!("════════════════════════════════════════════════════════════════════\n");

        let failed: Vec<&Row> = self.rows.iter().filter(|r| !r.pass).collect();
        assert!(
            failed.is_empty(),
            "{} acceptance check(s) failed: {}",
            failed.len(),
            failed
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
}

fn panic_message(p: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = p.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else {
        "non-string panic".to_string()
    }
}

// ── helpers over the loaded index ──────────────────────────────────────────────

/// Sorted fully-qualified paths of a symbol slice.
fn names(syms: &[Symbol]) -> Vec<String> {
    let mut v: Vec<String> = syms.iter().map(|s| s.fq_path.clone()).collect();
    v.sort();
    v
}

/// Assert every `wanted` path appears in `have`; return an evidence/reason string.
fn contains_all(have: &[Symbol], wanted: &[&str]) -> Result<String, String> {
    let set: BTreeSet<&str> = have.iter().map(|s| s.fq_path.as_str()).collect();
    let missing: Vec<&str> = wanted.iter().copied().filter(|w| !set.contains(w)).collect();
    if missing.is_empty() {
        Ok(format!("all {} present: {:?}", wanted.len(), wanted))
    } else {
        Err(format!(
            "missing {:?}; got {:?}",
            missing,
            names(have)
        ))
    }
}

/// The single symbol with exactly this fully-qualified path, via the C1 resolver.
fn find_sym(state: &IndexState, fq: &str) -> Option<Symbol> {
    state
        .resolve_symbol(fq, 1000)
        .data
        .into_iter()
        .find(|s| s.fq_path == fq)
}

// ── indexer setup ───────────────────────────────────────────────────────────────

/// `<repo>/` — parent of `crates/`.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("CARGO_MANIFEST_DIR has two ancestors")
        .to_path_buf()
}

/// Locate the prebuilt `callscope-index` binary, or fail with the build command.
fn locate_indexer(ws: &Path) -> PathBuf {
    if let Ok(p) = std::env::var("CALLSCOPE_INDEX_BIN") {
        let p = PathBuf::from(p);
        assert!(p.is_file(), "CALLSCOPE_INDEX_BIN does not point at a file: {}", p.display());
        return p;
    }
    let target = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| ws.join("target"));
    let candidates = [
        target.join("debug").join("callscope-index"),
        target.join("release").join("callscope-index"),
    ];
    let mut found: Option<PathBuf> = None;
    for c in candidates {
        if c.is_file() {
            // Prefer the more recently built one when both exist.
            let newer = match &found {
                None => true,
                Some(prev) => mtime(&c) >= mtime(prev),
            };
            if newer {
                found = Some(c);
            }
        }
    }
    found.unwrap_or_else(|| {
        panic!(
            "callscope-index binary not found under {}. Build it first:\n  \
             cargo build -p callscope-index\n\
             (or set CALLSCOPE_INDEX_BIN to its path)",
            target.display()
        )
    })
}

fn mtime(p: &Path) -> std::time::SystemTime {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .unwrap_or(std::time::UNIX_EPOCH)
}

/// Run `callscope-index <workspace>` once, returning the produced index bytes.
fn run_indexer(indexer: &Path, fixture: &Path) -> Vec<u8> {
    let status = Command::new(indexer)
        .arg(fixture)
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn {}: {e}", indexer.display()));
    assert!(
        status.success(),
        "callscope-index exited unsuccessfully ({status}) — cannot run acceptance without an index",
    );
    let index_path = fixture.join(".callscope").join("index.bin");
    std::fs::read(&index_path)
        .unwrap_or_else(|e| panic!("indexer did not produce {}: {e}", index_path.display()))
}

// ── the acceptance test ─────────────────────────────────────────────────────────

#[test]
fn acceptance_c1_c8_and_q1_q6() {
    let ws = workspace_root();
    let fixture = ws.join("fixtures").join("workspace");
    assert!(
        fixture.join("Cargo.toml").is_file(),
        "fixture workspace not found at {}",
        fixture.display()
    );
    let indexer = locate_indexer(&ws);

    // Build the index by running the REAL indexer, then run it a SECOND time to
    // check repeatable, deterministic indexing (Q5).
    eprintln!("acceptance: indexing fixture (run 1/2) with {}", indexer.display());
    let bytes_run1 = run_indexer(&indexer, &fixture);
    eprintln!("acceptance: indexing fixture (run 2/2) — determinism check (Q5)");
    let bytes_run2 = run_indexer(&indexer, &fixture);
    let deterministic = bytes_run1 == bytes_run2;

    // Load through the authoritative answer path (the MCP handler layer).
    let state = IndexState::load(&fixture).expect("load index.bin + manifest.json");

    let mut rep = Report::new();

    // ── C1: resolve returns the symbol + characteristics ────────────────────────
    rep.check("C1", "resolve a name to its symbol + characteristics", || {
        let env = state.resolve_symbol("normalize_token", DEFAULT_LIMIT);
        let sym = env
            .data
            .iter()
            .find(|s| s.fq_path == NORMALIZE)
            .ok_or_else(|| format!("normalize_token not resolved; got {:?}", names(&env.data)))?;
        if !sym.characteristics.public {
            return Err("normalize_token should be public".into());
        }
        Ok(format!(
            "resolve(\"normalize_token\") → {} (public={}, total={})",
            sym.fq_path, sym.characteristics.public, env.total
        ))
    });

    // ── C2: direct callers and callees ──────────────────────────────────────────
    rep.check("C2", "direct callers + callees of normalize_token", || {
        let id = SymbolId::from_fq_path(NORMALIZE);
        let env = state.direct_calls(id, DEFAULT_LIMIT);
        contains_all(
            &env.data.callers,
            &[
                "<parser::Simple as parser::Tokenizer>::tokenize",
                "<parser::Fancy as parser::Tokenizer>::tokenize",
                "parser::tests::normalizes_directly",
            ],
        )?;
        contains_all(&env.data.callees, &[ENSURE_ROUND_TRIP])?;
        Ok(format!(
            "callers={:?} callees={:?}",
            names(&env.data.callers),
            names(&env.data.callees)
        ))
    });

    // ── C3: transitive reachability, both directions ────────────────────────────
    rep.check("C3", "reachability forward + backward", || {
        let fwd = state.reachability(SymbolId::from_fq_path(RUN_GENERIC), Direction::Forward, DEFAULT_LIMIT);
        contains_all(&fwd.data, &[NORMALIZE, ENSURE_ROUND_TRIP])?;
        let back = state.reachability(SymbolId::from_fq_path(NORMALIZE), Direction::Backward, DEFAULT_LIMIT);
        contains_all(&back.data, &[RUN_DYN, RUN_GENERIC])?;
        Ok(format!(
            "forward(run_generic)→{} syms incl normalize_token+ensure_round_trip; \
             backward(normalize_token)→{} syms incl run_dyn+run_generic",
            fwd.total, back.total
        ))
    });

    // ── C4: enumerate call paths between two functions (end-to-end, not dead) ────
    rep.check("C4", "call_paths(run_generic → normalize_token) produces a real path", || {
        // Exercise the intended tool path (by-name → resolve → by-id).
        let by_name = state
            .call_paths_by_name("run_generic", "normalize_token", DEFAULT_MAX_DEPTH, DEFAULT_MAX_PATHS, DEFAULT_LIMIT)
            .map_err(|cands| format!("resolution failed, got candidates {:?}", names(&cands.data)))?;
        if by_name.data.is_empty() {
            return Err("call_paths_by_name returned zero paths — C4 not served".into());
        }
        let path = &by_name.data[0].nodes;
        let first = path.first().map(|s| s.fq_path.as_str()).unwrap_or("");
        let last = path.last().map(|s| s.fq_path.as_str()).unwrap_or("");
        if first != RUN_GENERIC || last != NORMALIZE {
            return Err(format!(
                "path does not run run_generic→…→normalize_token: {:?}",
                path.iter().map(|s| s.fq_path.clone()).collect::<Vec<_>>()
            ));
        }
        // Also exercise the by-id helper directly (the code the Turn-2 review
        // flagged as possibly dead): confirm it too produces the path.
        let by_id = state.call_paths(
            SymbolId::from_fq_path(RUN_GENERIC),
            SymbolId::from_fq_path(NORMALIZE),
            DEFAULT_MAX_DEPTH,
            DEFAULT_MAX_PATHS,
        );
        if by_id.data.is_empty() {
            return Err("by-id call_paths returned zero paths".into());
        }
        Ok(format!(
            "path = {:?} ({} path(s) by name, {} by id)",
            path.iter().map(|s| s.fq_path.clone()).collect::<Vec<_>>(),
            by_name.data.len(),
            by_id.data.len()
        ))
    });

    // ── C5 + Q1 (load-bearing §6 assertion): affected_tests includes BOTH the ────
    //    in-crate generic-dispatch test AND the separate-crate integration test.
    rep.check(
        "C5",
        "affected_tests(normalize_token) — the load-bearing §6 answer",
        || {
            let env = state.affected_tests(SymbolId::from_fq_path(NORMALIZE), DEFAULT_LIMIT);
            // The two load-bearing cases (Q1 + C5, the guiding example):
            contains_all(
                &env.data,
                &[
                    "parser::tests::reaches_via_generic_dispatch",
                    "integration::integration_reaches_via_generic",
                ],
            )
            .map_err(|e| format!("load-bearing generic-dispatch tests: {e}"))?;
            // …and the whole set of 8 affected tests.
            contains_all(&env.data, &EIGHT_AFFECTED_TESTS)?;
            if env.over_approximated.is_none() {
                return Err("affected_tests crosses dyn dispatch and must be over_approximated".into());
            }
            Ok(format!(
                "{} tests incl BOTH reaches_via_generic_dispatch (in-crate) AND \
                 integration_reaches_via_generic (separate crate); over_approximated={:?}",
                env.total, env.over_approximated
            ))
        },
    );

    // ── C6: reachable unsafe ─────────────────────────────────────────────────────
    rep.check("C6", "reachable_unsafe(normalize_token) finds the unsafe site", || {
        let env = state.reachable_unsafe(SymbolId::from_fq_path(NORMALIZE), DEFAULT_LIMIT);
        contains_all(&env.data, &[ENSURE_ROUND_TRIP])?;
        Ok(format!("reachable unsafe = {:?}", names(&env.data)))
    });

    // ── C7: combined impact = callers + affected tests ──────────────────────────
    rep.check("C7", "impact = direct callers + affected tests, in one answer", || {
        let env = state.impact(SymbolId::from_fq_path(NORMALIZE), DEFAULT_LIMIT);
        contains_all(
            &env.data.direct_callers,
            &["<parser::Simple as parser::Tokenizer>::tokenize", "parser::tests::normalizes_directly"],
        )
        .map_err(|e| format!("direct_callers: {e}"))?;
        contains_all(&env.data.affected_tests, &EIGHT_AFFECTED_TESTS)
            .map_err(|e| format!("affected_tests: {e}"))?;
        Ok(format!(
            "{} callers + {} affected tests (total={})",
            env.data.direct_callers.len(),
            env.data.affected_tests.len(),
            env.total
        ))
    });

    // ── C8: neighborhood graph is valid, safe Mermaid ───────────────────────────
    rep.check("C8", "neighborhood_graph renders valid Mermaid (safe ids, no \\n)", || {
        let env = state.neighborhood_graph(SymbolId::from_fq_path(NORMALIZE), DEFAULT_GRAPH_DEPTH, 60);
        let out = &env.data;
        if !out.starts_with("flowchart") {
            return Err(format!("not a flowchart: {out:.80}"));
        }
        // No literal backslash-n anywhere (a v11 parse error inside labels).
        if out.contains("\\n") {
            return Err("output contains a literal \\n, which v11 rejects".into());
        }
        // Every node-declaration id must be a safe generated form.
        for line in out.lines() {
            let t = line.trim_start();
            if let Some(brace) = t.find("[\"") {
                let node_id = &t[..brace];
                let safe = node_id == "trunc_note"
                    || ((node_id.starts_with('n') || node_id.starts_with('b'))
                        && node_id.len() > 1
                        && node_id[1..].chars().all(|c| c.is_ascii_digit()));
                if !safe {
                    return Err(format!("unsafe node id `{node_id}` in output"));
                }
            }
        }
        if !out.contains("classDef focus") {
            return Err("missing classDef declarations".into());
        }
        Ok(format!(
            "flowchart with {} node decl lines, safe ids, classDefs present",
            out.lines().filter(|l| l.trim_start().contains("[\"")).count()
        ))
    });

    // ── FIX-DYN: run_dyn over-approximates to ALL FOUR implementors ─────────────
    rep.check("DYN", "run_dyn dyn dispatch widens to all four implementors", || {
        let env = state.direct_calls(SymbolId::from_fq_path(RUN_DYN), DEFAULT_LIMIT);
        contains_all(&env.data.callees, &FOUR_IMPLEMENTORS)?;
        if env.over_approximated.is_none() {
            return Err("run_dyn's dyn edges must flag over_approximated".into());
        }
        Ok(format!(
            "run_dyn callees include Simple, Fancy, Wrapper<T> (generic), \
             ext_tokenizer::Shouty (cross-crate); over_approximated={:?}",
            env.over_approximated
        ))
    });

    // ── Q1: the four source-vs-execution gaps are closed ─────────────────────────
    rep.check("Q1-gen", "gap 1: generic instantiation resolves to the concrete impl", || {
        let sym = find_sym(&state, RUN_GENERIC)
            .ok_or_else(|| "monomorphized run_generic::<Simple> not in index".to_string())?;
        if !sym.characteristics.generic {
            return Err("run_generic::<Simple> should carry the generic characteristic".into());
        }
        // The monomorphized call resolves to Simple's concrete tokenize (static).
        let callees = state.direct_calls(sym.id, DEFAULT_LIMIT).data.callees;
        contains_all(&callees, &["<parser::Simple as parser::Tokenizer>::tokenize"])?;
        Ok("run_generic::<Simple> present and statically calls Simple::tokenize".into())
    });
    rep.check("Q1-dyn", "gap 2: dyn dispatch produces virtual edges", || {
        let env = state.direct_calls(SymbolId::from_fq_path(RUN_DYN), DEFAULT_LIMIT);
        // over_approximated is set only because the run_dyn edges are Virtual.
        if env.over_approximated.is_none() {
            return Err("run_dyn should produce virtual (dyn) edges".into());
        }
        Ok(format!("run_dyn → 4 virtual edges; {:?}", env.over_approximated))
    });
    rep.check("Q1-body", "gap 3: closure + async bodies fold into their enclosing fn", || {
        // Closure body: Simple::tokenize's `.map(|t| normalize_token(t))` closure
        // must fold back so Simple::tokenize statically calls normalize_token.
        let simple_id = SymbolId::from_fq_path("<parser::Simple as parser::Tokenizer>::tokenize");
        let simple_callees = state.direct_calls(simple_id, DEFAULT_LIMIT).data.callees;
        contains_all(&simple_callees, &[NORMALIZE])
            .map_err(|e| format!("closure body not folded into Simple::tokenize: {e}"))?;
        // Async body: tokenize_async's state-machine body must fold back so
        // tokenize_async reaches run_generic.
        let async_id = SymbolId::from_fq_path("parser::tokenize_async");
        let async_callees = state.direct_calls(async_id, DEFAULT_LIMIT).data.callees;
        contains_all(&async_callees, &[RUN_GENERIC])
            .map_err(|e| format!("async body not folded into tokenize_async: {e}"))?;
        Ok("closure folded into Simple::tokenize; async body folded into tokenize_async".into())
    });
    rep.check("Q1-test", "gap 4: macro-expanded #[test] / #[tokio::test] recognized", || {
        let plain = find_sym(&state, "parser::tests::normalizes_directly")
            .ok_or("plain #[test] not found")?;
        let tokio = find_sym(&state, "parser::tests::async_reaches_target")
            .ok_or("#[tokio::test] not found")?;
        if !plain.characteristics.test {
            return Err("plain #[test] not tagged as a test".into());
        }
        if !tokio.characteristics.test {
            return Err("#[tokio::test] not tagged as a test".into());
        }
        Ok("both #[test] and #[tokio::test] carry the test characteristic".into())
    });

    // ── Q2: an over-approximated (dyn) answer is flagged ─────────────────────────
    rep.check("Q2", "a dyn-dispatch answer is flagged over_approximated", || {
        let env = state.affected_tests(SymbolId::from_fq_path(NORMALIZE), DEFAULT_LIMIT);
        match &env.over_approximated {
            Some(reason) => Ok(format!("affected_tests flagged over_approximated: {reason:?}")),
            None => Err("dyn-reaching answer must set over_approximated".into()),
        }
    });

    // ── Q3: ambiguous name returns the candidate set, never a silent pick ────────
    rep.check("Q3", "ambiguous name returns candidates, never a silent pick", || {
        let env = state.resolve_symbol("tokenize", DEFAULT_LIMIT);
        if env.data.len() < 2 {
            return Err(format!("expected multiple candidates, got {:?}", names(&env.data)));
        }
        // A symbol-taking tool hands back the candidate set rather than guessing.
        let picked = state.resolve_then("tokenize", DEFAULT_LIMIT, |id| {
            state.direct_calls(id, DEFAULT_LIMIT)
        });
        match picked {
            Err(candidates) => Ok(format!(
                "\"tokenize\" is ambiguous → {} candidates returned, no pick made",
                candidates.data.len()
            )),
            Ok(_) => Err("ambiguous name was silently resolved to one symbol".into()),
        }
    });

    // ── Q4: a capped list reports its true total + truncated ─────────────────────
    rep.check("Q4", "a capped list reports true total and truncated", || {
        let env = state.affected_tests(SymbolId::from_fq_path(NORMALIZE), 3);
        if env.data.len() != 3 {
            return Err(format!("expected 3 returned, got {}", env.data.len()));
        }
        if env.total != 8 {
            return Err(format!("expected total=8, got {}", env.total));
        }
        if !env.truncated {
            return Err("capped list must set truncated=true".into());
        }
        Ok(format!("limit=3 → returned 3, total={}, truncated={}", env.total, env.truncated))
    });

    // ── Q5: repeatable, deterministic indexing ───────────────────────────────────
    rep.check("Q5", "re-running the indexer succeeds and is byte-deterministic", || {
        if !deterministic {
            return Err(format!(
                "two index runs differed ({} vs {} bytes)",
                bytes_run1.len(),
                bytes_run2.len()
            ));
        }
        Ok(format!(
            "two successive index runs produced byte-identical index.bin ({} bytes)",
            bytes_run1.len()
        ))
    });

    // ── Q6: editing a source file flips staleness ────────────────────────────────
    //    Done in an isolated temp copy so the committed fixture is never dirtied.
    rep.check("Q6", "editing a source file flips stale / reports diverged_files", || {
        q6_staleness_flips(&fixture)
    });

    // ── Boundary: the flag is SET where a third-party edge is crossed ────────────
    //    (Per the Turn-2 review, do NOT assert boundary_applies==false on forward
    //     answers — it fires on ordinary std calls. Assert it's set where a
    //     third-party edge is genuinely crossed.)
    rep.check("BND", "boundary_applies is set when a walk crosses into third-party code", || {
        let env = state.reachability(SymbolId::from_fq_path(ENSURE_ROUND_TRIP), Direction::Forward, DEFAULT_LIMIT);
        if !env.boundary_applies {
            return Err("forward walk from ensure_round_trip crosses std calls and should set boundary_applies".into());
        }
        Ok("forward walk from ensure_round_trip (which calls std) sets boundary_applies=true".into())
    });

    rep.finish();
}

/// Q6: copy the fixture to a temp dir with its freshly-produced index, confirm a
/// clean load is not stale, edit a `.rs` file, and confirm staleness flips and
/// names the edited file. Never touches the committed fixture.
fn q6_staleness_flips(fixture: &Path) -> Result<String, String> {
    let temp = std::env::temp_dir().join(format!(
        "callscope-acceptance-q6-{}-{}",
        std::process::id(),
        now_nanos()
    ));
    let _ = std::fs::remove_dir_all(&temp);
    copy_tree(fixture, &temp).map_err(|e| format!("copy fixture: {e}"))?;
    // Ensure the produced index artifacts are present in the copy.
    let dot = temp.join(".callscope");
    std::fs::create_dir_all(&dot).map_err(|e| format!("mk .callscope: {e}"))?;
    for f in ["index.bin", "manifest.json"] {
        std::fs::copy(fixture.join(".callscope").join(f), dot.join(f))
            .map_err(|e| format!("copy {f}: {e}"))?;
    }

    let result = (|| {
        let state = IndexState::load(&temp).map_err(|e| format!("load temp copy: {e}"))?;
        // Freshly copied: manifest matches disk → not stale.
        if state.compute_stale().map_err(|e| e.to_string())?.is_some() {
            return Err("a faithful copy of the indexed sources should not be stale".to_string());
        }
        // Edit a source file.
        let edited = temp.join("parser").join("src").join("lib.rs");
        let original = std::fs::read_to_string(&edited).map_err(|e| e.to_string())?;
        std::fs::write(&edited, format!("{original}\n// acceptance Q6 edit\n"))
            .map_err(|e| e.to_string())?;
        let stale = state
            .compute_stale()
            .map_err(|e| e.to_string())?
            .ok_or("index must be stale after editing a source file")?;
        if !stale.diverged_files.iter().any(|f| f.contains("lib.rs")) {
            return Err(format!("diverged_files should name the edited file: {:?}", stale.diverged_files));
        }
        Ok(format!(
            "clean copy not stale; after edit, stale with diverged_files={:?}",
            stale.diverged_files
        ))
    })();

    let _ = std::fs::remove_dir_all(&temp);
    result
}

fn now_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// Recursively copy `src` into `dst`, skipping `target/` and `.callscope/` (the
/// latter is re-populated with the index artifacts by the caller). Symlinks are
/// not followed.
fn copy_tree(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        let ft = entry.file_type()?;
        if ft.is_dir() {
            if name_str == "target" || name_str == ".callscope" {
                continue;
            }
            copy_tree(&entry.path(), &dst.join(&name))?;
        } else if ft.is_file() {
            std::fs::copy(entry.path(), dst.join(&name))?;
        }
        // Symlinks and other node types are skipped.
    }
    Ok(())
}
