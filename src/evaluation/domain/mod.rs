//! Pure domain types for the Evaluation context (no engine/infra deps).
//! Mirrors docs/ddd/contexts/evaluation.md.

pub mod metrics;
pub mod question;
pub mod question_result;
pub mod run;

pub use metrics::EvalMetrics;
pub use question::EvalQuestion;
pub use question_result::EvalQuestionResult;
pub use run::{EvalRun, EvalRunConfig, EvalRunStatus, ThresholdConfig};
