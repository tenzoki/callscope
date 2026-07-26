//! `parser` — the callscope fixture library crate.
//!
//! Every item here is arranged to exercise one of the four source-vs-execution
//! gaps in `problem.md` §2, so the callscope index has real, compiler-resolved
//! reachability to reproduce. The guiding-example target is
//! [`normalize_token`]: changing its signature must correctly implicate the
//! tests that reach it — including those that never spell its name.
//!
//! Reachability map (what actually calls `normalize_token` at run time):
//!
//! ```text
//!   normalize_token  ← Simple::tokenize   (via a closure body — gap 3)
//!                    ← Fancy::tokenize
//!                         ↑
//!   run_generic<T>  ─ t.tokenize ─┘        (monomorphized per T — gap 1)
//!   run_dyn         ─ (&dyn Tokenizer).tokenize  (virtual dispatch — gap 2)
//!   tokenize_async  ─ run_generic(&Simple, ..)   (async body — gap 3)
//!   normalize_token ─ ensure_round_trip           (reachable `unsafe` — C6)
//! ```

/// THE guiding-example target function (`problem.md` §2).
///
/// Lowercases and trims a single token. On its own reachability path it hits a
/// genuinely `unsafe` block (see [`ensure_round_trip`]), so it is also the
/// anchor for the "reachable unsafe" capability (C6).
pub fn normalize_token(t: &str) -> String {
    let lowered = t.to_lowercase();
    let round_tripped = ensure_round_trip(&lowered);
    round_tripped.trim().to_string()
}

/// A reachable `unsafe {}` block on a path from the public [`normalize_token`]
/// (C6). It is trivially safe in practice — a raw-pointer round-trip over the
/// string's own bytes, immediately re-validated — but genuinely `unsafe`, so
/// the unsafety check has something to attribute.
fn ensure_round_trip(s: &str) -> String {
    let bytes = s.as_bytes();
    let ptr = bytes.as_ptr();
    let len = bytes.len();
    // Reconstruct the exact same slice from its raw parts. Safe here because
    // `ptr`/`len` come straight from a live `&[u8]` we still hold, but the
    // reconstruction itself requires `unsafe`.
    let same: &[u8] = unsafe { std::slice::from_raw_parts(ptr, len) };
    String::from_utf8_lossy(same).into_owned()
}

/// The trait whose dispatch hides the interesting reachability.
pub trait Tokenizer {
    fn tokenize(&self, input: &str) -> Vec<String>;
}

/// First implementor. Its `tokenize` reaches [`normalize_token`] through a
/// **closure body** (`.map(|t| normalize_token(t))`) — gap 3, closure
/// attribution: the call textually sits in an anonymous closure, yet the
/// enclosing method is what reaches the target.
pub struct Simple;

impl Tokenizer for Simple {
    fn tokenize(&self, input: &str) -> Vec<String> {
        input
            .split_whitespace()
            .map(|t| normalize_token(t))
            .collect()
    }
}

/// Second implementor. Reaches [`normalize_token`] through an ordinary loop,
/// so trait dispatch has more than one concrete target to resolve.
pub struct Fancy;

impl Tokenizer for Fancy {
    fn tokenize(&self, input: &str) -> Vec<String> {
        let mut out = Vec::new();
        for raw in input.split(|c: char| c.is_whitespace() || c == ',') {
            if !raw.is_empty() {
                out.push(normalize_token(raw));
            }
        }
        out
    }
}

/// Generic entry point — the **monomorphization** site (gap 1). The single
/// source call `t.tokenize(input)` compiles to one concrete call per `T`, so a
/// caller instantiating `run_generic::<Simple>` thereby reaches
/// `Simple::tokenize` and, behind it, [`normalize_token`].
pub fn run_generic<T: Tokenizer>(t: &T, input: &str) -> Vec<String> {
    t.tokenize(input)
}

/// Trait-object entry point — the **`dyn` dispatch** site (gap 2). The honest
/// static answer for what this reaches is "any workspace implementor of
/// `Tokenizer`", which callscope reports as an over-approximated virtual edge.
pub fn run_dyn(t: &dyn Tokenizer, input: &str) -> Vec<String> {
    t.tokenize(input)
}

/// An `async fn` that transitively reaches [`normalize_token`] (gap 3, async
/// body). The compiler lowers this into a state machine detached from the
/// source function; callscope must fold that body back into `tokenize_async`.
pub async fn tokenize_async(input: &str) -> Vec<String> {
    run_generic(&Simple, input)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Names the target directly — the trivial "affected" case.
    #[test]
    fn normalizes_directly() {
        assert_eq!(normalize_token("  Hello "), "hello");
    }

    /// Reaches the target ONLY through generic trait dispatch — never spells
    /// `normalize_token`. This is the in-crate half of the load-bearing
    /// acceptance answer (§6): a source-level tool cannot connect this test to
    /// the target, but the compiler-grounded index can.
    #[test]
    fn reaches_via_generic_dispatch() {
        let out = run_generic(&Simple, "Foo Bar");
        assert_eq!(out, vec!["foo", "bar"]);
    }

    /// Reaches the target ONLY through `dyn` dispatch.
    #[test]
    fn reaches_via_dyn_dispatch() {
        let boxed: &dyn Tokenizer = &Fancy;
        let out = run_dyn(boxed, "Foo, Bar");
        assert_eq!(out, vec!["foo", "bar"]);
    }

    /// Macro-expanded test harness (`#[tokio::test]`, gap 4) whose async body
    /// transitively reaches the target through generic dispatch.
    #[tokio::test]
    async fn async_reaches_target() {
        let out = tokenize_async("Hello World").await;
        assert_eq!(out, vec!["hello", "world"]);
    }
}
