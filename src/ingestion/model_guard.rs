//! Write-side embedding-model guard for ingestion (PRD FR-EMB-002, Risk 5).
//!
//! Mirror of the retrieval read-side guard (R6/AC-7, see `retrieval::domain::errors`):
//! writing vectors from a model that differs from the one the existing index was built with
//! silently corrupts retrieval (mixed dimensions / incomparable spaces). This guard turns
//! that silent corruption into an explicit, early error at ingest time. `force` is the
//! deliberate opt-out for a full rebuild.

use crate::ingestion::domain::EmbeddingModelVersion;

/// Returns `Err(message)` when `incoming` is incompatible with the `existing` index model
/// and `force` is not set. Compatibility = identical `name` AND `dimension` (`created_at` is
/// ignored, matching the retrieval guard). An empty index (`existing == None`) is always
/// compatible — the first ingest defines the index model.
pub fn ensure_index_model_compatible(
    existing: Option<&EmbeddingModelVersion>,
    incoming: &EmbeddingModelVersion,
    force: bool,
) -> Result<(), String> {
    if force {
        return Ok(());
    }
    if let Some(existing) = existing {
        if existing.name != incoming.name || existing.dimension != incoming.dimension {
            return Err(format!(
                "embedding model mismatch: this index was built with '{}' (dim {}) but this run uses '{}' (dim {}). \
                 Re-ingest with --force to rebuild the scanned documents, or point --store at a fresh directory.",
                existing.name, existing.dimension, incoming.name, incoming.dimension
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(name: &str, dim: usize) -> EmbeddingModelVersion {
        EmbeddingModelVersion { name: name.into(), dimension: dim, created_at: "t".into() }
    }

    #[test]
    fn empty_index_is_always_compatible() {
        let incoming = model("mock", 8);
        assert!(ensure_index_model_compatible(None, &incoming, false).is_ok());
    }

    #[test]
    fn identical_model_is_compatible() {
        let existing = model("minilm", 384);
        let incoming = model("minilm", 384);
        assert!(ensure_index_model_compatible(Some(&existing), &incoming, false).is_ok());
    }

    #[test]
    fn created_at_difference_is_ignored() {
        let existing = EmbeddingModelVersion { name: "m".into(), dimension: 8, created_at: "2026".into() };
        let incoming = EmbeddingModelVersion { name: "m".into(), dimension: 8, created_at: "1970".into() };
        assert!(ensure_index_model_compatible(Some(&existing), &incoming, false).is_ok());
    }

    #[test]
    fn different_name_is_rejected() {
        let existing = model("minilm", 384);
        let incoming = model("mock-deterministic", 384);
        let err = ensure_index_model_compatible(Some(&existing), &incoming, false).unwrap_err();
        assert!(err.contains("minilm") && err.contains("mock-deterministic"), "got: {err}");
        assert!(err.contains("--force"), "should advertise the escape hatch: {err}");
    }

    #[test]
    fn different_dimension_is_rejected() {
        let existing = model("m", 384);
        let incoming = model("m", 8);
        let err = ensure_index_model_compatible(Some(&existing), &incoming, false).unwrap_err();
        assert!(err.contains("384") && err.contains('8'), "got: {err}");
    }

    #[test]
    fn force_bypasses_an_incompatible_model() {
        let existing = model("minilm", 384);
        let incoming = model("mock", 8);
        assert!(ensure_index_model_compatible(Some(&existing), &incoming, true).is_ok());
    }
}
