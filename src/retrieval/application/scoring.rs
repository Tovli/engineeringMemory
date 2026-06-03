//! Score normalization + tuning constants (ADR-0003).

/// Multiplier applied to `top_k` when filters are set, to defeat post-filter
/// truncation (ADR-0003 / spec E5). With no filters, exactly `top_k` is fetched.
pub const OVERFETCH: usize = 5;

/// Results below this similarity are still returned in M2 but counted in
/// `below_threshold_count` and flagged in `--explain` (spec E10). Tuned in M3.
pub const SIMILARITY_THRESHOLD: f32 = 0.30;

/// Map a cosine **distance** (lower = closer) to a similarity in [0,1]
/// (higher = better), matching the DDD `RetrievalResult.score` contract.
/// Clamped because non-unit-norm vectors can yield distance > 1 (ADR-0003 edge guard).
pub fn similarity_from_distance(distance: f32) -> f32 {
    (1.0 - distance).clamp(0.0, 1.0)
}

/// How many candidates to fetch from the store given the requested `top_k` and whether
/// filters are active. The store returns at most what it holds, so no index-size cap is
/// needed; over-fetch is bounded (`top_k * OVERFETCH`). Always at least 1.
pub fn fetch_k(top_k: usize, filters_set: bool) -> usize {
    let want = if filters_set { top_k.saturating_mul(OVERFETCH) } else { top_k };
    want.max(1)
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
    fn fetch_k_overfetches_only_when_filtering() {
        assert_eq!(fetch_k(8, false), 8); // no filter → exactly K
        assert_eq!(fetch_k(8, true), 40); // filter → K * OVERFETCH
        assert_eq!(fetch_k(0, false), 1); // never zero
    }
}
