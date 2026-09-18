pub mod corpus;
pub mod tool;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Bfs,
    Dfid,
    Simulation,
    #[default]
    Unknown,
}

/// Only verdicts explicitly reported by the oracle; absence of an error is not
/// evidence of success. `Unknown` includes interrupted and unsupported runs.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Success,
    Rejected,
    AssumptionViolated,
    InvariantViolated,
    InitialPropertyViolated,
    ActionPropertyViolated,
    TemporalPropertyViolated,
    Deadlock,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Answers {
    pub mode: Mode,
    pub outcome: Outcome,
    pub initial_distinct: Option<u64>,
    /// Observed count. Only a successful, completed BFS establishes a full count.
    pub distinct: Option<u64>,
    /// TLC's depth convention: initial states are at depth 1.
    pub depth: Option<u64>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Difference {
    Mode,
    Outcome,
    InitialDistinct,
    Distinct,
    Depth,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Comparison {
    /// Only the R0a summary agrees, not the entire answer contract.
    Match,
    Different(Vec<Difference>),
    Inconclusive,
}

/// Compare the available R0a contract. Partial exploration counts are deliberately
/// excluded: worker scheduling and which violation wins can change them.
pub fn compare(expected: &Answers, actual: &Answers) -> Comparison {
    if expected.outcome == Outcome::Unknown || actual.outcome == Outcome::Unknown {
        return Comparison::Inconclusive;
    }
    let mut differences = Vec::new();
    if expected.mode != actual.mode {
        differences.push(Difference::Mode);
    }
    if expected.outcome != actual.outcome {
        differences.push(Difference::Outcome);
    }
    if !differences.is_empty() {
        return Comparison::Different(differences);
    }
    // DFID and simulation have different completeness/depth semantics, outside R0a.
    if matches!(expected.mode, Mode::Dfid | Mode::Simulation) {
        return Comparison::Inconclusive;
    }
    if expected.outcome == Outcome::Success {
        if expected.mode != Mode::Bfs {
            return Comparison::Inconclusive;
        }
        for (field, left, right) in [
            (
                Difference::InitialDistinct,
                expected.initial_distinct,
                actual.initial_distinct,
            ),
            (Difference::Distinct, expected.distinct, actual.distinct),
            (Difference::Depth, expected.depth, actual.depth),
        ] {
            match (left, right) {
                (Some(left), Some(right)) if left != right => differences.push(field),
                (Some(_), Some(_)) => {}
                _ => return Comparison::Inconclusive,
            }
        }
    }
    if differences.is_empty() {
        Comparison::Match
    } else {
        Comparison::Different(differences)
    }
}

/// Canonical equality encoding; its derived order is not TLC's value order.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum ValueKey {
    Bool(bool),
    Int(i32),
    String(String),
    Model(String),
    Set(Vec<ValueKey>),
    Function(Vec<(ValueKey, ValueKey)>),
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub enum PropertyKind {
    Invariant,
    Initial,
    Action,
    Temporal,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct Property {
    pub kind: PropertyKind,
    pub name: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Verdict {
    Satisfied,
    Violated,
    EvaluationError,
    NotDetermined,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PropertyObservation {
    pub property: Property,
    pub verdict: Verdict,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ErrorCategory {
    ParseOrSemantic,
    Configuration,
    AssumptionEvaluation,
    Evaluation,
    Resource,
    Other,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RunError {
    pub category: ErrorCategory,
    pub code: i32,
    pub parameters: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReportedState {
    pub ordinal: i64,
    /// A detached display string. Values are not decoded by this extractor.
    pub display: String,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum TraceEnd {
    #[default]
    Finite,
    Stuttering {
        ordinal: i64,
    },
    BackTo {
        ordinal: i64,
    },
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Counterexample {
    /// Empty for deadlocks and evaluation failures without a named property.
    pub properties: Vec<Property>,
    pub states: Vec<ReportedState>,
    pub end: TraceEnd,
}

impl Counterexample {
    /// Check numbering and loop references only. Transition/property validity
    /// requires the evaluator and typed states; liveness also requires fairness.
    pub fn check_structure(&self) -> Result<(), String> {
        if self.states.is_empty() {
            return Err("counterexample has no states".into());
        }
        for (index, state) in self.states.iter().enumerate() {
            if state.ordinal != index as i64 + 1 {
                return Err(format!(
                    "expected state {}, found {}",
                    index + 1,
                    state.ordinal
                ));
            }
            if state.display.trim().is_empty() || state.display.starts_with("<unstringifiable ") {
                return Err(format!("state {} has no usable display", state.ordinal));
            }
        }
        let count = self.states.len() as i64;
        match self.end {
            TraceEnd::Stuttering { ordinal } if ordinal != count + 1 => {
                Err("stuttering marker must follow the last state".into())
            }
            TraceEnd::BackTo { ordinal } if !(1..=count).contains(&ordinal) => {
                Err("back-to-state marker points outside the trace".into())
            }
            _ => Ok(()),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ObservedRun {
    /// Original BFS trace records in report order; ordinal 1 starts a new trace.
    /// None means unavailable. Conversion failures remain explicit.
    #[serde(default)]
    pub typed_states: Option<Result<Vec<(i64, ValueKey)>, String>>,
    pub summary: Answers,
    pub configured_properties: Option<Vec<Property>>,
    /// Only observations actually reported. Absence is not a passing verdict.
    pub properties: Vec<PropertyObservation>,
    pub counterexamples: Vec<Counterexample>,
    pub error: Option<RunError>,
    pub unsupported: Vec<String>,
    pub exit_status: i32,
    pub deadlock_reported: bool,
}

impl ObservedRun {
    pub fn verdicts(&self) -> Option<Vec<PropertyObservation>> {
        self.configured_properties
            .as_ref()
            .map(|properties| self.project(properties.iter().cloned()))
    }

    /// Requires the same configured model's inventory. Unreported properties stay
    /// undetermined unless BFS completed successfully. This does not reload the model.
    pub fn verdicts_for(&self, model: &impl PropertyInventory) -> Vec<PropertyObservation> {
        self.project(model.properties().into_iter())
    }

    fn project(&self, properties: impl Iterator<Item = Property>) -> Vec<PropertyObservation> {
        let success = self.summary.mode == Mode::Bfs
            && self.summary.outcome == Outcome::Success
            && self.exit_status == 0
            && self.error.is_none()
            && self.unsupported.is_empty();
        properties
            .map(|property| {
                let verdict = self
                    .properties
                    .iter()
                    .find(|p| p.property == property)
                    .map(|p| p.verdict)
                    .unwrap_or(if success {
                        Verdict::Satisfied
                    } else {
                        Verdict::NotDetermined
                    });
                PropertyObservation { property, verdict }
            })
            .collect()
    }
}

pub trait PropertyInventory {
    fn properties(&self) -> Vec<Property>;
}

/// Faithful action identifier, meaningful only in the model that reported it.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ActionId(pub i32);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionInfo {
    pub id: ActionId,
    pub name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelInfo {
    /// Variable-slot order, used by the evaluator's state layout.
    pub variables: Vec<String>,
    pub initial_predicates: Vec<ActionInfo>,
    pub actions: Vec<ActionInfo>,
    pub has_next: bool,
    pub invariants: Vec<String>,
    pub initial_properties: Vec<String>,
    pub action_properties: Vec<String>,
    pub temporal_properties: Vec<String>,
    /// Temporal assumptions from SPECIFICATION, including fairness conjuncts.
    pub temporal_assumptions: Vec<String>,
    pub state_constraints: usize,
    pub action_constraints: usize,
    pub has_view: bool,
    pub has_symmetry: bool,
    pub check_deadlock: bool,
    pub assumptions: usize,
}

impl PropertyInventory for ModelInfo {
    fn properties(&self) -> Vec<Property> {
        [
            (PropertyKind::Invariant, &self.invariants),
            (PropertyKind::Initial, &self.initial_properties),
            (PropertyKind::Action, &self.action_properties),
            (PropertyKind::Temporal, &self.temporal_properties),
        ]
        .into_iter()
        .flat_map(|(kind, names)| {
            names.iter().map(move |name| Property {
                kind,
                name: name.clone(),
            })
        })
        .collect()
    }
}

#[derive(Clone, Debug)]
pub struct ModelRequest {
    pub spec: std::path::PathBuf,
    /// `None` selects the spec's sibling `.cfg`; an explicit relative path is
    /// resolved against the spec directory. A missing configuration is an error.
    pub config: Option<std::path::PathBuf>,
    /// Additional module directories, relative to the spec directory if needed.
    pub library_paths: Vec<std::path::PathBuf>,
    pub seed: i64,
    pub fingerprint_index: usize,
    pub community_modules: bool,
}

impl ModelRequest {
    pub fn new(spec: impl Into<std::path::PathBuf>) -> Self {
        Self {
            spec: spec.into(),
            config: None,
            library_paths: Vec::new(),
            seed: 1,
            fingerprint_index: 0,
            community_modules: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Stage {
    Session,
    Load,
    Assumptions,
    InitialStates,
    Successors,
    InspectState,
    Constraints,
    Properties,
    Canonicalization,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelError {
    pub stage: Stage,
    pub code: Option<i32>,
    pub message: String,
}

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.stage, self.message)
    }
}

impl std::error::Error for ModelError {}

impl ModelError {
    pub fn message(stage: Stage, message: impl ToString) -> Self {
        Self {
            stage,
            code: None,
            message: message.to_string(),
        }
    }
}

/// Display only; not a canonical encoding or extensional equality.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct StateSnapshot {
    pub values: std::collections::BTreeMap<String, String>,
}
