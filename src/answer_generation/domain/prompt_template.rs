//! PromptTemplate value object + the active, versioned template (FR-RAG-004 / R8, ADR-0007).
//! Templates live in source control with an immutable `version`; the version is stamped into
//! every answer log so prompt changes are auditable and regression-testable.

/// A versioned answer-generation prompt. `version` is immutable once shipped — changing the
/// prompt means a new `version` string (answer-generation.md PromptTemplate invariant).
#[derive(Debug, Clone, PartialEq)]
pub struct PromptTemplate {
    pub version: String,
    pub system_prompt: String,
    /// Body template; `{{chunks}}` and `{{question}}` are substituted by the renderer.
    pub context_template: String,
    /// Citation-format instructions appended after the body (enforces the SOURCES protocol, ADR-0008).
    pub instructions: String,
}

/// The current active prompt version. Bump on any change to the strings below (ADR-0007).
pub const ACTIVE_VERSION: &str = "v1.0.0";

/// The active template (ADR-0007: const-in-source, under version control).
pub fn active() -> PromptTemplate {
    PromptTemplate {
        version: ACTIVE_VERSION.to_string(),
        system_prompt: "You are tovli, a technical documentation assistant. Answer the question \
            using ONLY the provided sources. Do not use outside knowledge. If the sources do not \
            contain the answer, say that you don't have a reliable source."
            .to_string(),
        context_template: "Sources:\n{{chunks}}\n\nQuestion: {{question}}".to_string(),
        instructions: "Answer concisely. Cite every source you use by its chunk id. End your reply \
            with a line `SOURCES: <comma-separated chunk ids>` listing only the sources you used."
            .to_string(),
    }
}
