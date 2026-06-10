//! Score normalization + tuning constants (ADR-0003).

use crate::retrieval::domain::SearchMode;

/// Multiplier applied to `top_k` when filters are set, to defeat post-filter
/// truncation (ADR-0003 / spec E5). With no filters, exactly `top_k` is fetched.
pub const OVERFETCH: usize = 5;

/// Results below this similarity are still returned in M2 but counted in
/// `below_threshold_count` and flagged in `--explain` (spec E10). Tuned in M3.
pub const SIMILARITY_THRESHOLD: f32 = 0.30;
pub const RRF_K: f32 = 60.0;
pub const VECTOR_WEIGHT: f32 = 0.5;
pub const KEYWORD_WEIGHT: f32 = 0.5;

/// Map a cosine **distance** (lower = closer) to a similarity in [0,1]
/// (higher = better), matching the DDD `RetrievalResult.score` contract.
/// Clamped because non-unit-norm vectors can yield distance > 1 (ADR-0003 edge guard).
pub fn similarity_from_distance(distance: f32) -> f32 {
    (1.0 - distance).clamp(0.0, 1.0)
}

/// The ADR-0003 similarity threshold is calibrated only for vector cosine similarity.
/// Keyword and hybrid scores are mode-relative M5 relevance scores, so they should not
/// be interpreted with the vector threshold.
pub fn score_clears_similarity_threshold(mode: SearchMode, score: f32) -> bool {
    match mode {
        SearchMode::Vector => score >= SIMILARITY_THRESHOLD,
        SearchMode::Keyword | SearchMode::Hybrid => true,
    }
}

pub fn count_below_similarity_threshold<I>(mode: SearchMode, scores: I) -> usize
where
    I: IntoIterator<Item = f32>,
{
    match mode {
        SearchMode::Vector => scores
            .into_iter()
            .filter(|score| *score < SIMILARITY_THRESHOLD)
            .count(),
        SearchMode::Keyword | SearchMode::Hybrid => 0,
    }
}

/// How many candidates to fetch from the store given the requested `top_k` and whether
/// filters are active. The store returns at most what it holds, so no index-size cap is
/// needed; over-fetch is bounded (`top_k * OVERFETCH`). Always at least 1.
pub fn fetch_k(top_k: usize, filters_set: bool) -> usize {
    let want = if filters_set {
        top_k.saturating_mul(OVERFETCH)
    } else {
        top_k
    };
    want.max(1)
}

/// Candidate depth for hybrid search (ADR-0009): use the larger of filter over-fetch
/// and `top_k * 5`, always at least 5 so both modes have useful evidence.
pub fn hybrid_candidate_k(top_k: usize, filters_set: bool) -> usize {
    fetch_k(top_k, filters_set)
        .max(top_k.saturating_mul(OVERFETCH))
        .max(OVERFETCH)
}

/// Reciprocal Rank Fusion normalized to [0,1]. Ranks are 1-based; a missing
/// rank contributes zero (ADR-0009).
pub fn rrf_score(vector_rank: Option<usize>, keyword_rank: Option<usize>) -> f32 {
    let part = |rank: Option<usize>, weight: f32| -> f32 {
        rank.map(|r| weight / (RRF_K + r as f32)).unwrap_or(0.0)
    };
    let raw = part(vector_rank, VECTOR_WEIGHT) + part(keyword_rank, KEYWORD_WEIGHT);
    let max_raw = (VECTOR_WEIGHT + KEYWORD_WEIGHT) / (RRF_K + 1.0);
    if max_raw <= 0.0 {
        0.0
    } else {
        (raw / max_raw).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn similarity_is_inverted_and_clamped() {
        assert!((similarity_from_distance(0.0) - 1.0).abs() < 1e-6); // identical → best
        assert!((similarity_from_distance(1.0) - 0.0).abs() < 1e-6); // orthogonal → 0
        assert_eq!(similarity_from_distance(1.5), 0.0); // > 1 distance clamps to 0
        assert_eq!(similarity_from_distance(-0.2), 1.0); // < 0 distance clamps to 1
    }

    #[test]
    fn similarity_threshold_only_applies_to_vector_scores() {
        assert!(!score_clears_similarity_threshold(SearchMode::Vector, 0.29));
        assert!(score_clears_similarity_threshold(SearchMode::Vector, 0.30));
        assert!(score_clears_similarity_threshold(SearchMode::Keyword, 0.01));
        assert!(score_clears_similarity_threshold(SearchMode::Hybrid, 0.01));

        assert_eq!(
            count_below_similarity_threshold(SearchMode::Vector, [0.29, 0.30]),
            1
        );
        assert_eq!(
            count_below_similarity_threshold(SearchMode::Keyword, [0.01, 0.02]),
            0
        );
    }

    #[test]
    fn fetch_k_overfetches_only_when_filtering() {
        assert_eq!(fetch_k(8, false), 8); // no filter → exactly K
        assert_eq!(fetch_k(8, true), 40); // filter → K * OVERFETCH
        assert_eq!(fetch_k(0, false), 1); // never zero
    }

    #[test]
    fn hybrid_candidate_depth_keeps_extra_cross_mode_evidence() {
        assert_eq!(hybrid_candidate_k(8, false), 40);
        assert_eq!(hybrid_candidate_k(8, true), 40);
        assert_eq!(hybrid_candidate_k(1, false), 5);
        assert_eq!(hybrid_candidate_k(0, false), 5);
    }

    #[test]
    fn rrf_score_rewards_chunks_seen_by_both_modes() {
        let vector_only = rrf_score(Some(1), None);
        let keyword_only = rrf_score(None, Some(1));
        let both = rrf_score(Some(2), Some(1));

        assert!((vector_only - keyword_only).abs() < 1e-6);
        assert!(
            both > vector_only,
            "a chunk present in both candidate lists should rank higher"
        );
        assert!((rrf_score(Some(1), Some(1)) - 1.0).abs() < 1e-6);
        assert_eq!(rrf_score(None, None), 0.0);
    }
}
