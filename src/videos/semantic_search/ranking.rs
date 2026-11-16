//! Deterministic vector ranking for one embedding metric.

use crate::error::A2aLabError;
use crate::videos::model_space::EmbeddingMetric;
use crate::videos::semantic_search::result::SemanticRetrievalScore;

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
}
