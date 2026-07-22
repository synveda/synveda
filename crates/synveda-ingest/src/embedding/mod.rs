//! The Embedder seam (MEM-4, ADR-0023 decision 6): one trait, two
//! implementations — text-embeddings-inference (TEI, the product path,
//! tech plan §1.3) and a deterministic hash embedder that keeps dev,
//! demos, and tests network-free (seed §2.1) — dispatched statically
//! through [`AnyEmbedder`], the `AnyExtractor` shape.
//!
//! The worker calls [`Embedder::embed`] per event, outside any
//! transaction; the vectors then commit atomically with their records
//! under the archive-lock. A failure here is a pre-commit failure like
//! any other: the signal redelivers (ADR-0022 decision 6).

use synveda_types::Result;

mod deterministic;
mod tei;

pub use deterministic::DeterministicEmbedder;
pub use tei::TeiEmbedder;

/// The embedding seam. Implementations must return exactly one vector
/// per input, in order, and report failures as
/// [`synveda_types::Error::Dependency`] so the worker leaves the signal
/// for its visibility-timeout retry — never a partial result.
///
/// `async fn` in a public trait is deliberate: dispatch is static
/// through [`AnyEmbedder`], never `dyn` (the [`crate::extraction`]
/// precedent).
#[allow(async_fn_in_trait)]
pub trait Embedder {
    /// The stable method name recorded in metrics labels and audit
    /// payloads (`tei`, `deterministic`).
    fn method(&self) -> &'static str;

    /// The model identity recorded on every embedding row.
    fn model(&self) -> &str;

    /// Embeds each input into one vector, preserving order.
    async fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>>;
}

/// The configured embedder, dispatched statically (no `dyn`, no boxed
/// futures — the worker holds exactly one for its lifetime).
#[derive(Debug, Clone)]
pub enum AnyEmbedder {
    /// The hash-based, network-free default (seed §2.1: zero-config).
    /// Its geometry is meaningless noise — an honest placeholder,
    /// never a retrieval substrate (ADR-0023 decision 6).
    Deterministic(DeterministicEmbedder),
    /// text-embeddings-inference serving BGE-M3 or compatible — the
    /// product path (tech plan §1.3).
    Tei(TeiEmbedder),
}

impl Embedder for AnyEmbedder {
    fn method(&self) -> &'static str {
        match self {
            AnyEmbedder::Deterministic(inner) => inner.method(),
            AnyEmbedder::Tei(inner) => inner.method(),
        }
    }

    fn model(&self) -> &str {
        match self {
            AnyEmbedder::Deterministic(inner) => inner.model(),
            AnyEmbedder::Tei(inner) => inner.model(),
        }
    }

    async fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>> {
        match self {
            AnyEmbedder::Deterministic(inner) => inner.embed(inputs).await,
            AnyEmbedder::Tei(inner) => inner.embed(inputs).await,
        }
    }
}
