use sha2::{Digest, Sha256};

use crate::error::LoongMemoryError;

pub trait EmbeddingProvider: Send + Sync {
    fn dimension(&self) -> usize;
    fn embed(&self, text: &str) -> Result<Vec<f32>, LoongMemoryError>;
}

#[derive(Debug, Clone)]
pub struct DeterministicHashEmbedder {
    dim: usize,
}

impl DeterministicHashEmbedder {
    pub fn new(dim: usize) -> Self {
        Self { dim: dim.max(8) }
    }
}

impl Default for DeterministicHashEmbedder {
    fn default() -> Self {
        Self::new(256)
    }
}

impl EmbeddingProvider for DeterministicHashEmbedder {
    fn dimension(&self) -> usize {
        self.dim
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>, LoongMemoryError> {
        let mut out = vec![0.0_f32; self.dim];
        for token in text.split_whitespace() {
            let mut hasher = Sha256::new();
            hasher.update(token.as_bytes());
            let bytes = hasher.finalize();
            for chunk in bytes.chunks(4) {
                if chunk.len() < 4 {
                    continue;
                }
                let idx = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) as usize
                    % self.dim;
                out[idx] += 1.0;
            }
        }
        let norm = out.iter().map(|v| v * v).sum::<f32>().sqrt();
        if norm > 0.0 {
            for v in &mut out {
                *v /= norm;
            }
        }
        Ok(out)
    }
}
