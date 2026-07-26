//! Integration test target — compiles as its own crate, separate from the
//! `parser` library (problem.md §6: "a test from a different crate's
//! integration-test target").
//!
//! Every test here reaches `parser::normalize_token` ONLY through generic
//! trait dispatch (via `run_generic`) or `dyn` dispatch (via `run_dyn`). None
//! of them names `normalize_token`. This is the load-bearing acceptance
//! answer: the affected-tests result for `normalize_token` must include these
//! even though no source-level search could connect them.

use parser::{run_dyn, run_generic, Fancy, Simple, Tokenizer};

/// Reaches the target purely through generic monomorphization (gap 1), from a
/// separate integration-test crate. This is the primary acceptance answer.
#[test]
fn integration_reaches_via_generic() {
    let out = run_generic(&Simple, "Alpha Beta");
    assert_eq!(out, vec!["alpha", "beta"]);
}

/// Reaches the target purely through `dyn` dispatch (gap 2), also from the
/// separate crate.
#[test]
fn integration_reaches_via_dyn() {
    let t: &dyn Tokenizer = &Fancy;
    let out = run_dyn(t, "Gamma, Delta");
    assert_eq!(out, vec!["gamma", "delta"]);
}
