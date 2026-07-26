//! Per-crate call-graph extraction over monomorphized MIR (`rustc_public` /
//! `stable_mir`).
//!
//! This module runs *inside* the `rustc_public` context that
//! [`crate::driver`] establishes for one crate compilation. It produces a
//! [`Fragment`] — the symbols and edges this one compilation can see. The
//! `callscope-index` orchestrator merges every crate's fragment into the final
//! index ([`crate::merge`]).
//!
//! # How the four gaps close here
//!
//! - **Gap 1 (generic instantiation).** Every call terminator's callee is
//!   resolved with [`Instance::resolve`], which is the compiler's own
//!   monomorphization: a call inside `run_generic::<Simple>` resolves to the
//!   concrete `Simple::tokenize`, not to the trait method. That resolved
//!   `Instance`'s specialized name (e.g. `parser::run_generic::<parser::Simple>`)
//!   is the symbol id, so edges join up across crate fragments by content.
//! - **Gap 2 (`dyn` dispatch).** When resolution yields an
//!   [`InstanceKind::Virtual`] callee (or a trait-method call that cannot be
//!   resolved to one concrete impl), the callee is over-approximated to every
//!   workspace implementor of the trait, and each edge is tagged
//!   [`EdgeKind::Virtual`] so the query layer flags the answer.
//! - **Gap 3 (closure / async bodies).** A closure or coroutine body is folded
//!   into its enclosing user function: its calls are attributed to the parent
//!   symbol, not to an anonymous closure symbol.
//! - **Gap 4 (characteristics).** `test`/`public` come from the compiler via the
//!   internal `TyCtxt` (`#[rustc_test_marker]` and the visibility query);
//!   `async`/`generic`/`foreign` come from stable `rustc_public` queries;
//!   `uses_unsafe` is derived from MIR (an `unsafe fn`, or any call to a
//!   function whose signature is `unsafe` — which is only legal inside an
//!   `unsafe` block).

use std::collections::{HashMap, HashSet, VecDeque};

use rustc_middle::ty::TyCtxt;
use rustc_public::mir::mono::{Instance, InstanceKind};
use rustc_public::mir::{Body, Safety, TerminatorKind};
use rustc_public::ty::{AssocKind, FnDef, GenericArgKind, GenericArgs, RigidTy, Ty, TyKind};
use rustc_public::{
    all_local_items, local_crate, rustc_internal, CrateDef, CrateDefType, CrateItem, DefId,
    ItemKind,
};
use serde::{Deserialize, Serialize};

use callscope_core::{Characteristics, Edge, EdgeKind, Span, Symbol, SymbolId};

/// The symbols and edges extracted from one crate compilation.
#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Fragment {
    pub symbols: Vec<Symbol>,
    pub edges: Vec<Edge>,
}

/// Extract a [`Fragment`] from the crate currently being compiled.
///
/// `tcx` is the internal compiler context, used only for the two
/// characteristics `rustc_public` does not expose cleanly (`test`, `public`).
pub fn extract(tcx: TyCtxt<'_>) -> Fragment {
    let mut b = Builder::new(tcx);
    b.run();
    b.into_fragment()
}

struct Builder<'tcx> {
    tcx: TyCtxt<'tcx>,
    /// Symbols keyed by fully-qualified path, deduplicated within this fragment.
    ///
    /// Keyed by `fq_path` rather than by [`SymbolId`] on purpose: if two
    /// distinct paths ever hashed to the same id, an id-keyed map would drop one
    /// silently here — before the orchestrator's `merge` (which owns
    /// fail-on-collision) could ever see the conflict. Keying by path keeps both
    /// symbols in the emitted fragment so the collision surfaces there.
    symbols: HashMap<String, Symbol>,
    /// Edges deduplicated by (from, to, kind).
    edges: HashSet<(u64, u64, u8)>,
    edge_list: Vec<Edge>,
    /// Instances already walked (by specialized name), to avoid re-walking and
    /// to terminate on cycles.
    walked: HashSet<String>,
    /// Closure/coroutine def ids already recursed into (fold termination).
    folded: HashSet<u64>,
    queue: VecDeque<Instance>,
    /// Crate-relative paths of every `#[test]` function in this crate (e.g.
    /// `tests::normalizes_directly`), harvested from the harness-generated
    /// `#[rustc_test_marker]` consts. See [`Builder::collect_test_markers`].
    test_paths: HashSet<String>,
}

impl<'tcx> Builder<'tcx> {
    fn new(tcx: TyCtxt<'tcx>) -> Self {
        Builder {
            tcx,
            symbols: HashMap::new(),
            edges: HashSet::new(),
            edge_list: Vec::new(),
            walked: HashSet::new(),
            folded: HashSet::new(),
            queue: VecDeque::new(),
            test_paths: HashSet::new(),
        }
    }

    fn into_fragment(self) -> Fragment {
        Fragment {
            symbols: self.symbols.into_values().collect(),
            edges: self.edge_list,
        }
    }

    /// Seed roots (every non-generic local function) and drain the worklist.
    fn run(&mut self) {
        self.collect_test_markers();
        for item in all_local_items() {
            if item.kind() != ItemKind::Fn {
                continue;
            }
            // Generic definitions are not roots; they enter the graph only as
            // concrete instances resolved at a call site (gap 1).
            if item.requires_monomorphization() {
                continue;
            }
            if let Ok(inst) = Instance::try_from(item) {
                self.enqueue(inst);
            }
        }
        while let Some(inst) = self.queue.pop_front() {
            self.walk_instance(inst);
        }
    }

    /// Enqueue an instance for walking if we have not seen it and it has a body.
    fn enqueue(&mut self, inst: Instance) {
        // Only user-defined function items become workspace symbols. A resolved
        // shim (e.g. the closure-call shim behind `Iterator::map`'s closure),
        // an intrinsic, or a vtable-`Virtual` instance has no user `fq_path`,
        // span, or ordinary function signature — `FnDef::fn_sig` panics on the
        // closure type behind such a shim — and is not something a change-impact
        // query reasons about. Skip them; their bodies (for closures) are folded
        // into the owning user function via `fold_def_body` instead.
        if !matches!(inst.kind, InstanceKind::Item) {
            return;
        }
        if !inst.has_body() {
            return;
        }
        let name = inst.name();
        if self.walked.contains(&name) {
            return;
        }
        self.queue.push_back(inst);
    }

    fn walk_instance(&mut self, inst: Instance) {
        let name = inst.name();
        if !self.walked.insert(name.clone()) {
            return;
        }
        let id = SymbolId::from_fq_path(&name);
        // Record the symbol for this instance (it is a workspace function: we
        // only ever walk instances local to the crate being compiled).
        let sym = self.symbol_for(&inst, &name, id);
        self.symbols.entry(name.clone()).or_insert(sym);

        if let Some(body) = inst.body() {
            let mut uses_unsafe = false;
            self.walk_body(&body, id, &mut uses_unsafe);
            if uses_unsafe {
                if let Some(s) = self.symbols.get_mut(&name) {
                    s.characteristics.uses_unsafe = true;
                }
            }
        }
    }

    /// Walk one body, attributing every discovered edge to `owner`. Closure and
    /// coroutine bodies recurse here with the *same* `owner` (gap 3 folding).
    fn walk_body(&mut self, body: &Body, owner: SymbolId, uses_unsafe: &mut bool) {
        let locals = body.locals();

        // Gap 3: fold every closure / coroutine used by this body into `owner`.
        //
        // Detection is by the *types* flowing through the body's locals, not by
        // an `Aggregate` construction rvalue. A non-capturing closure is a
        // zero-sized type whose construction is elided, so it never appears as
        // an `Aggregate(Closure)` — the old construction-site scan silently
        // missed it, which is why `Simple::tokenize` (whose target is reached
        // only through a `.map(|t| normalize_token(t))` closure) lost its edge
        // to `normalize_token` while `Fancy::tokenize` (a plain loop) kept it.
        //
        // The closure survives instead as a *type*: it is handed to
        // `Iterator::map`, so it lives nested inside the adapter's type,
        // `std::iter::Map<_, {closure}>`, in one of this body's locals. A
        // recursive search of every local's type therefore catches capturing
        // and non-capturing closures alike, and folds their bodies into the
        // concrete `owner` instance. `fold_def_body`'s own `folded` set makes
        // the fold idempotent.
        let mut closure_defs = Vec::new();
        let mut seen_tys = HashSet::new();
        for local in locals {
            collect_closures(local.ty, &mut seen_tys, &mut closure_defs);
        }
        for def_id in closure_defs {
            self.fold_def_body(def_id, owner, uses_unsafe);
        }

        // Resolve every call terminator's concrete callee.
        for block in &body.blocks {
            if let TerminatorKind::Call { func, .. } = &block.terminator.kind {
                let Ok(func_ty) = func.ty(locals) else { continue };
                let kind = func_ty.kind();
                let Some((fndef, args)) = kind.fn_def() else { continue };
                self.handle_call(fndef, args, owner, uses_unsafe);
            }
        }
    }

    /// Fold a closure/coroutine body (identified by its def id) into `owner`.
    fn fold_def_body(&mut self, def_id: DefId, owner: SymbolId, uses_unsafe: &mut bool) {
        let internal = rustc_internal::internal(self.tcx, def_id);
        let key = ((internal.krate.as_u32() as u64) << 32) | (internal.index.as_u32() as u64);
        if !self.folded.insert(key) {
            return;
        }
        // Reconstruct a def with a body. Closures and coroutines both expose a
        // MIR body via their def id.
        if let Some(body) = body_of_def(def_id) {
            self.walk_body(&body, owner, uses_unsafe);
        }
    }

    /// Emit the edge(s) for one resolved call.
    fn handle_call(
        &mut self,
        fndef: FnDef,
        args: &GenericArgs,
        owner: SymbolId,
        uses_unsafe: &mut bool,
    ) {
        // Calling an `unsafe`-signature function is only legal inside an
        // `unsafe` block, so it is evidence the owner uses unsafe (gap 4, C6).
        // Read the signature safely: `fn_sig` panics on any def whose type is
        // not an ordinary function type, so go through the optional accessor.
        if fn_sig_is_unsafe(fndef) {
            *uses_unsafe = true;
        }

        match Instance::resolve(fndef, args) {
            Ok(callee) if !is_virtual(&callee) => {
                // Static, concrete callee (gap 1). Only a real user function
                // item is a call target worth an edge; a resolved shim or
                // intrinsic (e.g. a closure-call shim) is not a workspace
                // symbol, so it gets neither an edge nor a walk.
                if matches!(callee.kind, InstanceKind::Item) {
                    let name = callee.name();
                    let to = SymbolId::from_fq_path(&name);
                    self.add_edge(owner, to, EdgeKind::Static);
                    if is_local_instance(&callee) {
                        self.enqueue(callee);
                    }
                }
            }
            Ok(_) => {
                // Resolved to a virtual (vtable) instance: a `dyn` call.
                self.emit_virtual(fndef, owner);
            }
            Err(_) => {
                // Could not resolve to one concrete instance. If this is a trait
                // method, treat it as a `dyn`/over-approximated call (gap 2);
                // otherwise it is a call we cannot ground, so we drop it.
                if fndef.associated_item().is_some() {
                    self.emit_virtual(fndef, owner);
                }
            }
        }
    }

    /// Over-approximate a `dyn`/unresolved trait-method call to every workspace
    /// implementor of the trait, each as a [`EdgeKind::Virtual`] edge (gap 2).
    fn emit_virtual(&mut self, trait_method: FnDef, owner: SymbolId) {
        let Some(assoc) = trait_method.associated_item() else { return };
        let method_name = match &assoc.kind {
            AssocKind::Fn { name, .. } => name.clone(),
            _ => return,
        };
        // The trait is the parent definition of the trait method.
        let Some(trait_did) = trait_method.def_id().parent() else { return };

        for impl_def in local_crate().trait_impls() {
            let tr = impl_def.trait_impl();
            if tr.value.def_id.def_id() != trait_did {
                continue;
            }
            for ai in impl_def.associated_items() {
                let AssocKind::Fn { name, .. } = &ai.kind else { continue };
                if *name != method_name {
                    continue;
                }
                let impl_fn = FnDef(ai.def_id.def_id());
                if let Ok(inst) = Instance::resolve(impl_fn, &GenericArgs(vec![])) {
                    let to = SymbolId::from_fq_path(&inst.name());
                    self.add_edge(owner, to, EdgeKind::Virtual);
                    if is_local_instance(&inst) {
                        self.enqueue(inst);
                    }
                }
            }
        }
    }

    fn add_edge(&mut self, from: SymbolId, to: SymbolId, kind: EdgeKind) {
        if from == to {
            return; // drop trivial self-loops from folding
        }
        let disc = match kind {
            EdgeKind::Static => 0u8,
            EdgeKind::Virtual => 1u8,
        };
        if self.edges.insert((from.0, to.0, disc)) {
            self.edge_list.push(Edge { from, to, kind });
        }
    }

    /// Build the [`Symbol`] for a walked instance, tagging its characteristics.
    fn symbol_for(&self, inst: &Instance, name: &str, id: SymbolId) -> Symbol {
        let def_id = inst.def.def_id();
        let fndef = FnDef(def_id);

        let crate_name = CrateItem::try_from(*inst)
            .ok()
            .map(|it| it.krate().name)
            .unwrap_or_else(|| name.split("::").next().unwrap_or("").to_string());

        let span = span_of(def_id);

        let generic = CrateItem::try_from(*inst)
            .map(|it| it.requires_monomorphization())
            .unwrap_or(false);

        let characteristics = Characteristics {
            test: self.is_test(name),
            public: self.is_public(def_id),
            is_async: fndef.asyncness().is_async(),
            generic,
            foreign: inst.is_foreign_item(),
            // An `unsafe fn` declares its unsafety in its own signature. Read it
            // safely (see `fn_sig_is_unsafe`); reachable `unsafe {}` blocks are
            // added later while walking the body.
            uses_unsafe: fn_sig_is_unsafe(fndef),
        };

        Symbol {
            id,
            fq_path: name.to_string(),
            crate_name,
            span,
            characteristics,
        }
    }

    /// `#[test]` / `#[tokio::test]` detection.
    ///
    /// In test mode the harness expands `#[test] fn foo` into the untouched `fn
    /// foo` *plus* a generated `const foo: test::TestDescAndFn` (see
    /// `rustc_builtin_macros::test`). The historical detection read a
    /// `#[rustc_test_marker]` attribute, but that marker (a) rides the generated
    /// const, never the fn, and (b) is now a *parsed* builtin attribute that
    /// `TyCtxt::get_attrs` no longer surfaces by name — so it came back empty
    /// and every symbol was mis-tagged `test: false`, silently emptying the C5
    /// `affected_tests` answer.
    ///
    /// Two independent facts identify the harness const with no reliance on
    /// attribute plumbing: its type is the `test::TestDescAndFn` ADT, and it
    /// carries the *same fully-qualified path as the test fn* — a path a
    /// `const`/`fn` pair can share only because the harness synthesised it, never
    /// in ordinary source. We collect those const paths; a function symbol is a
    /// test iff its own path is among them.
    fn collect_test_markers(&mut self) {
        for item in all_local_items() {
            if item.kind() != ItemKind::Const {
                continue;
            }
            if let TyKind::RigidTy(RigidTy::Adt(adt, _)) = item.ty().kind() {
                if adt.name().ends_with("test::TestDescAndFn") {
                    self.test_paths.insert(item.name());
                }
            }
        }
    }

    /// Whether the instance named `name` is a `#[test]` function: its
    /// fully-qualified path matches a harvested `TestDescAndFn` const.
    fn is_test(&self, name: &str) -> bool {
        self.test_paths.contains(name)
    }

    fn is_public(&self, def_id: DefId) -> bool {
        let internal = rustc_internal::internal(self.tcx, def_id);
        self.tcx.visibility(internal).is_public()
    }
}

/// Whether an instance's definition lives in the crate currently being
/// compiled. We only descend into local instances; calls into other crates are
/// emitted as edges (their target crate's own compilation supplies the rest).
fn is_local_instance(inst: &Instance) -> bool {
    inst.def.krate().is_local
}

fn is_virtual(inst: &Instance) -> bool {
    matches!(inst.kind, InstanceKind::Virtual { .. })
}

/// Whether a function definition's own signature is declared `unsafe`.
///
/// `FnDef::fn_sig` unwraps `self.ty().kind().fn_sig()`, which is `None` for any
/// def whose type is not an ordinary function type (a closure, a shim, an
/// unusual synthetic def). Go through the optional accessor so a non-function
/// def yields `false` instead of panicking the whole compilation.
fn fn_sig_is_unsafe(fndef: FnDef) -> bool {
    fndef
        .ty()
        .kind()
        .fn_sig()
        .map(|sig| sig.value.safety == Safety::Unsafe)
        .unwrap_or(false)
}

/// Get the MIR body for a closure or coroutine def id.
fn body_of_def(def_id: DefId) -> Option<Body> {
    // `ClosureDef` and `CoroutineDef` each wrap a `DefId` and expose `.body()`;
    // we route through whichever produces a body. `CoroutineClosureDef` (async
    // closures) exposes no stable body accessor, so such a def simply yields
    // `None` and is not folded — none appears in the fixture.
    use rustc_public::ty::{ClosureDef, CoroutineDef};
    ClosureDef(def_id)
        .body()
        .or_else(|| CoroutineDef(def_id).body())
}

/// Recursively collect the def ids of every closure / coroutine appearing
/// anywhere in `ty`, including nested inside generic arguments.
///
/// A non-capturing closure handed to a generic higher-order function survives
/// only as a type parameter of the adapter it was passed to (e.g.
/// `std::iter::Map<_, {closure}>`), never as a top-level local type, so a
/// shallow match would miss it. The `seen` set over `Ty` (which is `Copy + Eq +
/// Hash`) guards against recursive types such as `enum List { Cons(Box<List>) }`.
fn collect_closures(ty: Ty, seen: &mut HashSet<Ty>, out: &mut Vec<DefId>) {
    if !seen.insert(ty) {
        return;
    }
    let TyKind::RigidTy(rigid) = ty.kind() else {
        return;
    };
    match rigid {
        RigidTy::Closure(def, args) => {
            out.push(def.def_id());
            collect_closures_in_args(&args, seen, out);
        }
        RigidTy::Coroutine(def, args) => {
            out.push(def.def_id());
            collect_closures_in_args(&args, seen, out);
        }
        RigidTy::CoroutineClosure(def, args) => {
            out.push(def.def_id());
            collect_closures_in_args(&args, seen, out);
        }
        RigidTy::Adt(_, args)
        | RigidTy::FnDef(_, args)
        | RigidTy::CoroutineWitness(_, args) => {
            collect_closures_in_args(&args, seen, out);
        }
        RigidTy::Ref(_, inner, _) | RigidTy::RawPtr(inner, _) => {
            collect_closures(inner, seen, out);
        }
        RigidTy::Array(inner, _) | RigidTy::Slice(inner) | RigidTy::Pat(inner, _) => {
            collect_closures(inner, seen, out);
        }
        RigidTy::Tuple(tys) => {
            for t in tys {
                collect_closures(t, seen, out);
            }
        }
        _ => {}
    }
}

fn collect_closures_in_args(args: &GenericArgs, seen: &mut HashSet<Ty>, out: &mut Vec<DefId>) {
    for arg in &args.0 {
        if let GenericArgKind::Type(t) = arg {
            collect_closures(*t, seen, out);
        }
    }
}

fn span_of(def_id: DefId) -> Span {
    let sp = def_id.span();
    let lines = sp.get_lines();
    Span {
        file: sp.get_filename(),
        line_start: lines.start_line as u32,
        line_end: lines.end_line as u32,
    }
}
