//! The deterministic hash embedder: BLAKE3 of the content expanded to a
//! fixed 16-dimension L2-normalised vector (ADR-0023 decision 6). Same
//! content, same vector, no network, no model download — the
//! zero-config default for dev, demos, and tests. Its geometry carries
//! no meaning: equal texts collide, similar texts do not attract, and
//! CTX-1's retrieval-quality work never runs against it.

use synveda_types::Result;

use super::Embedder;

/// The fixed output dimension.
const DIM: usize = 16;

/// The hash-based embedder.
#[derive(Debug, Clone, Default)]
pub struct DeterministicEmbedder;

impl DeterministicEmbedder {
    /// The model identity recorded on rows this embedder writes.
    pub const MODEL: &'static str = "hash@1";

    /// Builds the embedder.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

/// One content string's vector: hash bytes taken pairwise as signed
/// 16-bit values, then L2-normalised.
fn embed_one(input: &str) -> Vec<f32> {
    let digest = blake3::hash(input.as_bytes());
    let bytes = digest.as_bytes();
    let mut vector: Vec<f32> = (0..DIM)
        .map(|i| f32::from(i16::from_le_bytes([bytes[2 * i], bytes[2 * i + 1]])))
        .collect();
    let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in &mut vector {
            *value /= norm;
        }
    } else {
        // Unreachable in practice (a BLAKE3 digest is never all zeros),
        // but a zero vector would be a degenerate row: pin a unit axis.
        vector[0] = 1.0;
    }
    vector
}

impl Embedder for DeterministicEmbedder {
    fn method(&self) -> &'static str {
        "deterministic"
    }

    fn model(&self) -> &str {
        Self::MODEL
    }

    async fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(inputs.iter().map(|input| embed_one(input)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn vectors_are_deterministic_normalised_and_input_sensitive() {
        let embedder = DeterministicEmbedder::new();
        let inputs = vec!["alpha".to_owned(), "beta".to_owned(), "alpha".to_owned()];
        let vectors = embedder.embed(&inputs).await.expect("embed");
        assert_eq!(vectors.len(), 3);
        for vector in &vectors {
            assert_eq!(vector.len(), DIM);
            let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
            assert!((norm - 1.0).abs() < 1e-5, "unit norm, got {norm}");
        }
        assert_eq!(vectors[0], vectors[2], "same content, same vector");
        assert_ne!(
            vectors[0], vectors[1],
            "different content, different vector"
        );
    }

    #[tokio::test]
    async fn empty_input_yields_empty_output() {
        let embedder = DeterministicEmbedder::new();
        assert!(embedder.embed(&[]).await.expect("embed").is_empty());
    }
}
