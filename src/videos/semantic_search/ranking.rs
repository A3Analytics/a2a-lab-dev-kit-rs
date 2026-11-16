//! Deterministic vector ranking and reciprocal-rank fusion.

use std::cmp::Ordering;
use std::collections::HashSet;

use crate::error::A2aLabError;
use crate::videos::model_space::EmbeddingMetric;
use crate::videos::semantic_search::result::SemanticRetrievalScore;

/// Reciprocal-rank fusion constant *k*.
///
/// Lane contribution is `1 / (RRF_K + rank)` for each 1-based lane rank.
/// Classic RRF uses `k = 60` so rank-1 scores `1/61` and later ranks decay slowly.
pub const RRF_K: f32 = 60.0;

/// Similarity used for ordering: higher is always better.
pub fn rank_value(
    metric: EmbeddingMetric,
    query: &[f32],
    candidate: &[f32],
) -> Result<f32, A2aLabError> {
    if query.len() != candidate.len() {
        return Err(A2aLabError::invalid(
            "embedding",
            "query and candidate dimensions must match",
        ));
    }
    let score = match metric {
        EmbeddingMetric::Cosine | EmbeddingMetric::Dot => dot(query, candidate),
        EmbeddingMetric::Euclidean => -euclidean(query, candidate),
    };
    if !score.is_finite() {
        return Err(A2aLabError::invalid(
            "retrieval_score",
            "must be a finite number",
        ));
    }
    Ok(score)
}

/// Public retrieval score in the declared metric.
pub fn retrieval_score(
    metric: EmbeddingMetric,
    query: &[f32],
    candidate: &[f32],
) -> Result<SemanticRetrievalScore, A2aLabError> {
    let value = match metric {
        EmbeddingMetric::Cosine | EmbeddingMetric::Dot => dot(query, candidate),
        EmbeddingMetric::Euclidean => euclidean(query, candidate),
    };
    SemanticRetrievalScore::new(value, metric)
}

/// Case-insensitive lexical strength against persisted description text.
///
/// `1.0` when the trimmed query is a substring of the description, `0.5` when
/// at least one alphanumeric query token matches a whole alphanumeric
/// description token, otherwise `0.0`.
pub fn lexical_strength(query: &str, description: &str) -> f32 {
    let query = query.trim();
    if query.is_empty() {
        return 0.0;
    }
    let query_folded = query.to_lowercase();
    let description_folded = description.to_lowercase();
    if description_folded.contains(&query_folded) {
        return 1.0;
    }
    let query_tokens = alphanumeric_tokens(&query_folded);
    if query_tokens.is_empty() {
        return 0.0;
    }
    let description_tokens: HashSet<&str> = alphanumeric_tokens(&description_folded)
        .into_iter()
        .collect();
    if query_tokens
        .iter()
        .any(|token| description_tokens.contains(token))
    {
        0.5
    } else {
        0.0
    }
}

/// Whether trimmed text contains at least one alphanumeric token.
#[must_use]
pub fn has_alphanumeric_token(text: &str) -> bool {
    text.split(|ch: char| !ch.is_alphanumeric())
        .any(|part| !part.is_empty())
}

/// FTS5 MATCH query of alphanumeric tokens, or `None` when none exist.
#[must_use]
pub fn fts5_match_query(text: &str) -> Option<String> {
    let tokens = alphanumeric_tokens(text.trim());
    if tokens.is_empty() {
        return None;
    }
    Some(
        tokens
            .into_iter()
            .map(|token| format!("\"{token}\""))
            .collect::<Vec<_>>()
            .join(" OR "),
    )
}

/// Reciprocal-rank fusion of optional 1-based lexical BM25/FTS and vector ranks.
///
/// Missing lanes contribute `0`. Rank `0` is treated as missing.
#[must_use]
pub fn reciprocal_rank_fusion(lexical_rank: Option<u32>, vector_rank: Option<u32>) -> f32 {
    rrf_lane(lexical_rank) + rrf_lane(vector_rank)
}

/// Exact-match class first (`1.0` > `0.5` > `0.0`), then higher RRF.
#[must_use]
pub fn cmp_fused_rank(
    left_strength: f32,
    left_rrf: f32,
    right_strength: f32,
    right_rrf: f32,
) -> Ordering {
    left_strength
        .total_cmp(&right_strength)
        .reverse()
        .then_with(|| left_rrf.total_cmp(&right_rrf).reverse())
}

/// Vector candidates need `score_floor`; lexical matches bypass that floor.
#[must_use]
pub fn admits_hybrid_candidate(
    semantic_rank: f32,
    lexical_strength: f32,
    score_floor: f32,
) -> bool {
    lexical_strength > 0.0 || semantic_rank >= score_floor
}

fn rrf_lane(rank: Option<u32>) -> f32 {
    let Some(rank) = rank else {
        return 0.0;
    };
    let rank = u16::try_from(rank).unwrap_or(u16::MAX);
    if rank == 0 {
        return 0.0;
    }
    1.0 / (RRF_K + f32::from(rank))
}

fn alphanumeric_tokens(folded: &str) -> Vec<&str> {
    folded
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect()
}

fn dot(left: &[f32], right: &[f32]) -> f32 {
    left.iter()
        .zip(right.iter())
        .map(|(left, right)| left * right)
        .sum()
}

fn euclidean(left: &[f32], right: &[f32]) -> f32 {
    left.iter()
        .zip(right.iter())
        .map(|(left, right)| {
            let delta = left - right;
            delta * delta
        })
        .sum::<f32>()
        .sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_ranks_identical_vectors_highest() {
        let query = [1.0, 0.0];
        let match_hit = rank_value(EmbeddingMetric::Cosine, &query, &[1.0, 0.0]).unwrap();
        let other = rank_value(EmbeddingMetric::Cosine, &query, &[0.0, 1.0]).unwrap();
        assert!(match_hit > other);
    }

    #[test]
    fn euclidean_rank_value_prefers_nearer_vectors() {
        let query = [0.0, 0.0];
        let near = rank_value(EmbeddingMetric::Euclidean, &query, &[0.1, 0.0]).unwrap();
        let far = rank_value(EmbeddingMetric::Euclidean, &query, &[1.0, 0.0]).unwrap();
        assert!(near > far);
    }

    #[test]
    fn lexical_strength_substring_token_and_none() {
        assert!(
            (lexical_strength(" red ", "A RED pipette on the deck") - 1.0).abs() < f32::EPSILON
        );
        assert!((lexical_strength("red tube", "red pipette") - 0.5).abs() < f32::EPSILON);
        assert!(lexical_strength("monkey", "A red pipette on the deck").abs() < f32::EPSILON);
        assert!(lexical_strength("!!!", "A red pipette on the deck").abs() < f32::EPSILON);
        assert!(!has_alphanumeric_token("!!!"));
        assert!(has_alphanumeric_token("red"));
        assert_eq!(
            fts5_match_query(" red tube ").as_deref(),
            Some("\"red\" OR \"tube\"")
        );
        assert!(fts5_match_query("!!!").is_none());
        assert!(admits_hybrid_candidate(0.05, 1.0, 0.99));
        assert!(!admits_hybrid_candidate(0.05, 0.0, 0.99));
        assert!(admits_hybrid_candidate(0.99, 0.0, 0.99));
    }

    #[test]
    fn rrf_uses_named_k_and_one_based_ranks() {
        assert!((RRF_K - 60.0).abs() < f32::EPSILON);
        let rank1 = reciprocal_rank_fusion(Some(1), None);
        let rank2 = reciprocal_rank_fusion(Some(2), None);
        assert!((rank1 - 1.0 / 61.0).abs() < 1e-6);
        assert!((rank2 - 1.0 / 62.0).abs() < 1e-6);
        assert!(rank1 > rank2);
        let both = reciprocal_rank_fusion(Some(1), Some(1));
        assert!((both - 2.0 / 61.0).abs() < 1e-6);
        assert!(reciprocal_rank_fusion(Some(0), None).abs() < f32::EPSILON);
        assert!(reciprocal_rank_fusion(None, None).abs() < f32::EPSILON);
    }

    #[test]
    fn additive_hybrid_inverts_across_metrics_rrf_does_not() {
        const LEXICAL_MATCH_BOOST: f32 = 1.0;
        let additive = |semantic_rank: f32, lexical_strength: f32| {
            semantic_rank + LEXICAL_MATCH_BOOST * lexical_strength
        };
        let lexical_cosine = 0.1;
        let vector_cosine = 0.99;
        let lexical_euclidean = -5.0;
        let vector_euclidean = -0.1;
        assert!(additive(lexical_cosine, 1.0) > additive(vector_cosine, 0.0));
        assert!(additive(lexical_euclidean, 1.0) < additive(vector_euclidean, 0.0));

        let lexical_rrf = reciprocal_rank_fusion(Some(1), None);
        let vector_rrf = reciprocal_rank_fusion(None, Some(1));
        assert!((lexical_rrf - vector_rrf).abs() < f32::EPSILON);
        assert_eq!(
            cmp_fused_rank(1.0, lexical_rrf, 0.0, vector_rrf),
            Ordering::Less
        );
        assert_eq!(
            cmp_fused_rank(1.0, lexical_rrf, 0.5, vector_rrf),
            Ordering::Less
        );
        assert_eq!(
            cmp_fused_rank(0.5, lexical_rrf, 0.0, vector_rrf),
            Ordering::Less
        );
    }

    #[test]
    fn public_retrieval_score_stays_metric_similarity() {
        let query = [1.0, 0.0];
        let cosine = retrieval_score(EmbeddingMetric::Cosine, &query, &[1.0, 0.0]).unwrap();
        let euclidean = retrieval_score(EmbeddingMetric::Euclidean, &query, &[0.0, 0.0]).unwrap();
        assert!((cosine.value() - 1.0).abs() < 1e-6);
        assert_eq!(cosine.metric(), EmbeddingMetric::Cosine);
        assert!((euclidean.value() - 1.0).abs() < 1e-6);
        assert_eq!(euclidean.metric(), EmbeddingMetric::Euclidean);
        let fused = reciprocal_rank_fusion(Some(1), Some(2));
        assert!((cosine.value() - fused).abs() > 0.5);
    }
}
