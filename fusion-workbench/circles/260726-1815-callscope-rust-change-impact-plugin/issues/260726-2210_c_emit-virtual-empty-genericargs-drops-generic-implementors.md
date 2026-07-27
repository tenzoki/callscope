dyn over-approximation drops generic implementors and generic trait methods (empty GenericArgs in emit_virtual)
---
`callscope-index` over-approximates a `dyn`/unresolved trait-method call to every
workspace implementor by resolving each impl method with **no generic arguments**:
`crates/callscope-index/src/graph_build.rs:299` —
`Instance::resolve(impl_fn, &GenericArgs(vec![]))`. This resolves only for impl
methods that need zero generic arguments (a non-generic impl of a trait with a
non-generic method, e.g. `impl Tokenizer for Simple`). For a generic implementor
(`impl<T> Tokenizer for Wrapper<T>`) or a generic trait method (`fn tokenize<U>()`),
resolution requires the missing type args, so `Instance::resolve` returns `Err`,
the `if let Ok(inst)` arm is skipped, and **no virtual edge is emitted** for that
implementor.
---
Severity: Medium. Calibration: inference — read from source, not run. I could
not empirically confirm against a real workspace because building the nightly
compiler-linked crate to construct a generic-implementor fixture was out of scope
for this review.

Why it matters: this is a soundness gap in the *honest over-approximation* that
is the tool's core value (Q1/Q2). The README (`README.md:117-121`) claims a
dyn answer widens to "every workspace implementor" and "none that could fire is
missing." With this defect, a generic implementor that could fire at run time is
silently absent from `affected_tests` / `reachability` / `impact` — an
under-approximation presented as the honest superset. That is exactly the
"silent best guess is a defect" case Q2 forbids.

Does NOT block P11 acceptance: the fixture's implementors (`Simple`, `Fancy`)
are non-generic unit structs in a single crate, so this path never triggers on
the acceptance substrate. It is a real-workspace generality gap, safe to defer
past acceptance — but the README over-claim should be softened until it is fixed.

Fix direction: build the correct `GenericArgs` for each impl method before
`Instance::resolve` (the impl's own substitutions), or, where the concrete args
are unknowable at a `dyn` site, record the implementor as an over-approximated
edge without requiring a fully-monomorphized instance. Whichever path is chosen,
a generic implementor must not vanish from the widened set.

Affects: callscope-index (P4), `graph_build.rs::emit_virtual`. Cross-ref README
honesty note in this review.

---
Resolved: Closed. On the pinned nightly the old
`Instance::resolve(impl_fn, &GenericArgs(vec![]))` did not silently drop a
generic implementor — it returned a POLYMORPHIC instance whose `body()` panicked
("has parameters, but no args were provided in instantiate"), aborting the whole
crate compilation once a generic implementor existed. Fixed in
`crates/callscope-index/src/graph_build.rs`: `collect_impls` detects a generic
impl (`ImplDef::generics_of().params` non-empty) and calls `walk_generic_method`,
which names the implementor by its polymorphic method path
(`<parser::Wrapper<T> as parser::Tokenizer>::tokenize`), flags the symbol
`generic`, and walks its body via `FnDef::body()` (item-level `mir_body`, which
does NOT monomorphize and does not panic). The `dyn` over-approximation includes
it via the merge-time join, and the walked body genuinely reaches
`parser::normalize_token` (through its folded `.map(|t| normalize_token(t))`
closure). Verified: `run_dyn` widens to `Wrapper<T>::tokenize`, and
`affected_tests(normalize_token)` now includes `parser::tests::reaches_via_dyn_wrapper`.
Residual (single polymorphic node rather than per-monomorphization, visibly
flagged) documented in
decisions/260726-2253_a_generic-implementor-over-approximation-representation.md.
