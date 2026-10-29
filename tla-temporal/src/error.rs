use std::fmt;

/// Why a temporal property could not be instantiated or checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Evaluating a predicate, a quantifier domain or a successor failed.
    Evaluation(tlaplus_model::Error),
    /// Liveness needs the complete state graph.
    IncompleteExploration,
    /// A property names a definition the program does not have.
    MissingPredicate(String),
    /// A report and the model it came from disagree.
    InconsistentReport(&'static str),
    /// The property search over the behavior graph failed.
    Search(tlaplus_props::Error),
    /// The product of graph and formula grew past the configured limit.
    ProductLimit,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Evaluation(e) => e.fmt(f),
            Self::IncompleteExploration => {
                f.write_str("liveness requires complete safety exploration")
            }
            Self::MissingPredicate(name) => write!(f, "missing temporal predicate {name}"),
            Self::InconsistentReport(what) => f.write_str(what),
            Self::Search(e) => e.fmt(f),
            Self::ProductLimit => {
                f.write_str("liveness product limit reached; result is incomplete")
            }
        }
    }
}

impl std::error::Error for Error {}

impl From<tlaplus_model::Error> for Error {
    fn from(e: tlaplus_model::Error) -> Self {
        Self::Evaluation(e)
    }
}

impl From<tlaplus_values::Error> for Error {
    fn from(e: tlaplus_values::Error) -> Self {
        Self::Evaluation(e.into())
    }
}
