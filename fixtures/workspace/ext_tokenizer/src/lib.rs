//! `ext_tokenizer` — a second workspace member whose only job is to define a
//! **cross-crate implementor** of `parser`'s `Tokenizer` trait (gap 2c).
//!
//! The `Tokenizer` trait and the `&dyn Tokenizer` call site (`parser::run_dyn`)
//! both live in the `parser` crate. `Shouty` implements the trait here, in a
//! *different* member crate. When `parser` is compiled, its `dyn` over-
//! approximation can only see `parser`'s own implementors; `Shouty` is visible
//! only when the whole workspace's implementors are enumerated. That is exactly
//! the workspace-wide join the indexer must perform.
//!
//! `Shouty::tokenize` reaches `parser::normalize_token` (a cross-crate static
//! call), so a change to `normalize_token` must implicate any test that drives
//! `Shouty` through `&dyn Tokenizer`.

use parser::{normalize_token, Tokenizer};

/// Cross-crate implementor of `parser::Tokenizer`. Upper-cases nothing on its
/// own — it simply splits and normalizes, reaching `parser::normalize_token`.
pub struct Shouty;

impl Tokenizer for Shouty {
    fn tokenize(&self, input: &str) -> Vec<String> {
        input
            .split_whitespace()
            .map(|t| normalize_token(t))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parser::run_dyn;

    /// Drives the cross-crate implementor `Shouty` through `parser::run_dyn`'s
    /// `&dyn Tokenizer` site. The coercion happens here, in `ext_tokenizer`,
    /// but the dyn call site itself is compiled in `parser` — so only a
    /// workspace-wide implementor join emits the edge
    /// `parser::run_dyn -> <ext_tokenizer::Shouty as parser::Tokenizer>::tokenize`.
    #[test]
    fn cross_crate_reaches_via_dyn() {
        let boxed: &dyn Tokenizer = &Shouty;
        let out = run_dyn(boxed, "Foo Bar");
        assert_eq!(out, vec!["foo", "bar"]);
    }
}
