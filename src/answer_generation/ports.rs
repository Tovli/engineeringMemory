//! Ports (seams) for the Answer Generation context. The only outbound dependency is the LLM,
//! wrapped by `LlmPort` (ACL). Infra implements it; application depends on the trait, never on a
//! concrete provider. The domain holds no SDK clients or API keys (ADR-0006, PRD §15 Risk 3).

/// Why the LLM stopped generating. `Error` is treated by the service as a provider failure (E6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishReason {
    Stop,
    Length,
    Error,
}

/// A rendered prompt ready to send to the LLM. Strings only — no chunk objects leak across the seam.
#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub system_prompt: String,
    pub user_message: String,
    pub max_tokens: usize,
}

/// The LLM's reply. `cited_chunk_ids` is the structured citation channel (ADR-0008): the adapter
/// parses the model's `SOURCES:` line into chunk ids; the service then validates them against the run.
#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub text: String,
    pub cited_chunk_ids: Vec<String>,
    pub finish_reason: FinishReason,
    pub provider: String,
    pub latency_ms: u128,
}

/// Anti-corruption layer over the LLM provider. The domain trusts the LLM only for fluent text,
/// never for the correctness of sources — citation validity is enforced by the service (ADR-0006).
pub trait LlmPort {
    /// Generate an answer for the rendered request. `Err` (or `finish_reason == Error`) is a
    /// provider failure → the service produces `NoAnswerReason::LlmProviderError`.
    fn complete(&self, request: &LlmRequest) -> anyhow::Result<LlmResponse>;

    /// Whether the provider is usable right now. When `false`, the service never calls `complete`.
    fn is_available(&self) -> bool;
}
