//! The single output shape every MCP tool returns.
//!
//! Q2 (visible uncertainty), Q4 (bounded output with totals), Q6 (staleness),
//! and the dependency boundary are not per-tool special cases; they are fields
//! on one [`Envelope`] that every answer carries. Keeping them here, in one
//! type, is what stops the eight-tool surface from fragmenting into eight
//! differently-shaped results.
//!
//! This is a plain data type. The only logic is construction helpers — nothing
//! here decides *whether* a result is stale or over-approximated; the query and
//! staleness modules do that and fill the fields in.

use serde::{Deserialize, Serialize};

/// What the staleness check found changed since the index was built (Q6).
///
/// Present on an [`Envelope`] only when the fingerprint check failed. The
/// listed files are the ones whose content diverged from the manifest, so the
/// agent can see exactly what went out of date rather than a bare "stale" flag.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct StaleInfo {
    pub diverged_files: Vec<String>,
}

impl StaleInfo {
    pub fn new(diverged_files: Vec<String>) -> Self {
        StaleInfo { diverged_files }
    }
}

/// Why a result over-approximates (Q2).
///
/// `#[non_exhaustive]` so new sources of over-approximation can be added without
/// breaking downstream matches — a consumer such as `callscope-mcp` must carry a
/// wildcard arm. The canonical case is `dyn` dispatch: a `Virtual` call edge is
/// widened to every workspace implementor of the trait, so the answer names a
/// superset of what actually executes.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Reason {
    /// A `dyn`-dispatch call was widened to all workspace implementors of the
    /// trait. `implementor_count` is how many the answer folds in.
    DynDispatch {
        trait_path: String,
        implementor_count: usize,
    },
}

impl Reason {
    /// Convenience constructor for the `dyn`-dispatch case.
    pub fn dyn_dispatch(trait_path: impl Into<String>, implementor_count: usize) -> Self {
        Reason::DynDispatch {
            trait_path: trait_path.into(),
            implementor_count,
        }
    }
}

/// The one shape every tool returns.
///
/// The uncertainty fields (`stale`, `over_approximated`) are `Option` and are
/// omitted from the serialized JSON when absent, so "no uncertainty" reads as
/// the field simply not being there rather than an explicit null. The flag
/// fields (`truncated`, `boundary_applies`) and `total` are always present.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Envelope<T> {
    /// The tool's actual result.
    pub data: T,
    /// Present when the index is stale relative to the current sources (Q6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stale: Option<StaleInfo>,
    /// Present when the answer includes more than what provably executes (Q2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub over_approximated: Option<Reason>,
    /// True when a size bound cut the result short (Q4). `total` still reports
    /// the full count.
    pub truncated: bool,
    /// Total number of results before any truncation (Q4).
    pub total: usize,
    /// True when the answer's walk crossed the workspace boundary into a
    /// third-party crate, which v1 does not descend into.
    pub boundary_applies: bool,
}

impl<T> Envelope<T> {
    /// An exact answer: no staleness, no over-approximation, not truncated, no
    /// boundary crossing. `total` starts at 0; set it with [`Envelope::with_total`]
    /// when the payload is a collection whose size the caller wants to report.
    pub fn exact(data: T) -> Self {
        Envelope {
            data,
            stale: None,
            over_approximated: None,
            truncated: false,
            total: 0,
            boundary_applies: false,
        }
    }

    /// Set the total result count (Q4).
    pub fn with_total(mut self, total: usize) -> Self {
        self.total = total;
        self
    }

    /// Attach staleness information (Q6).
    pub fn with_stale(mut self, info: StaleInfo) -> Self {
        self.stale = Some(info);
        self
    }

    /// Attach an over-approximation reason (Q2).
    pub fn with_over_approximation(mut self, reason: Reason) -> Self {
        self.over_approximated = Some(reason);
        self
    }

    /// Set the truncation flag (Q4).
    pub fn with_truncated(mut self, truncated: bool) -> Self {
        self.truncated = truncated;
        self
    }

    /// Set the boundary-crossing flag.
    pub fn with_boundary(mut self, boundary_applies: bool) -> Self {
        self.boundary_applies = boundary_applies;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_omits_uncertainty_fields_when_serialized() {
        let env = Envelope::exact(vec!["a".to_string(), "b".to_string()]).with_total(2);
        let json = serde_json::to_string(&env).expect("serialize");

        // Absent uncertainty is omitted, not serialized as null.
        assert!(!json.contains("stale"), "stale must be omitted when None: {json}");
        assert!(
            !json.contains("over_approximated"),
            "over_approximated must be omitted when None: {json}",
        );
        // Flags and total are always present.
        assert!(json.contains("\"truncated\":false"), "{json}");
        assert!(json.contains("\"boundary_applies\":false"), "{json}");
        assert!(json.contains("\"total\":2"), "{json}");
    }

    #[test]
    fn uncertainty_fields_present_when_set() {
        let env = Envelope::exact(vec![1u32, 2, 3])
            .with_total(7)
            .with_truncated(true)
            .with_boundary(true)
            .with_stale(StaleInfo::new(vec!["src/parser.rs".to_string()]))
            .with_over_approximation(Reason::dyn_dispatch("parser::Tokenizer", 2));
        let json = serde_json::to_string(&env).expect("serialize");

        assert!(json.contains("diverged_files"), "{json}");
        assert!(json.contains("src/parser.rs"), "{json}");
        assert!(json.contains("DynDispatch"), "{json}");
        assert!(json.contains("parser::Tokenizer"), "{json}");
        assert!(json.contains("\"truncated\":true"), "{json}");
        assert!(json.contains("\"boundary_applies\":true"), "{json}");
    }

    #[test]
    fn envelope_round_trips_through_serde() {
        let env = Envelope::exact(vec![10u32, 20])
            .with_total(2)
            .with_over_approximation(Reason::dyn_dispatch("parser::Tokenizer", 3));
        let json = serde_json::to_string(&env).expect("serialize");
        let round: Envelope<Vec<u32>> = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(env, round);
    }
}
