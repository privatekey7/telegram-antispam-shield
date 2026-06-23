//! Linear spam classifier loaded from a compact binary file.
//!
//! Binary format (little-endian), produced by `scripts/train_model.py`:
//!
//! ```text
//! magic[4] = "SASM"
//! version: u32
//! num_buckets: u32
//! char_min: u8, char_max: u8, word_ngram_max: u8, _pad: u8
//! bias: f32
//! threshold: f32
//! weights: num_buckets * f32
//! ```

use anyhow::{bail, Context, Result};
use std::path::Path;

use crate::features;

/// Model bytes baked into the binary so deployment needs no extra files.
/// Override at runtime by setting `MODEL_PATH` to a custom-trained model.
const EMBEDDED_MODEL: &[u8] = include_bytes!("../model/spam_model.bin");

const HEADER_LEN: usize = 24;
const MAGIC: &[u8; 4] = b"SASM";

/// A trained linear classifier over hashed text features.
pub struct Model {
    pub num_buckets: u64,
    pub char_min: usize,
    pub char_max: usize,
    pub word_ngram_max: u8,
    pub bias: f32,
    pub threshold: f32,
    weights: Vec<f32>,
}

impl Model {
    /// Parse a model from raw bytes.
    pub fn from_bytes(buf: &[u8]) -> Result<Self> {
        if buf.len() < HEADER_LEN {
            bail!("model file too small ({} bytes)", buf.len());
        }
        if &buf[0..4] != MAGIC {
            bail!("bad magic: not a SASM model");
        }
        let version = u32::from_le_bytes(buf[4..8].try_into().unwrap());
        if version != 1 {
            bail!("unsupported model version {version}");
        }
        let num_buckets = u32::from_le_bytes(buf[8..12].try_into().unwrap()) as u64;
        let char_min = buf[12] as usize;
        let char_max = buf[13] as usize;
        let word_ngram_max = buf[14];
        let bias = f32::from_le_bytes(buf[16..20].try_into().unwrap());
        let threshold = f32::from_le_bytes(buf[20..24].try_into().unwrap());

        let expected = HEADER_LEN + (num_buckets as usize) * 4;
        if buf.len() != expected {
            bail!(
                "weights length mismatch: got {}, expected {}",
                buf.len(),
                expected
            );
        }

        let weights: Vec<f32> = buf[HEADER_LEN..]
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();

        Ok(Self {
            num_buckets,
            char_min,
            char_max,
            word_ngram_max,
            bias,
            threshold,
            weights,
        })
    }

    /// Load the embedded model, or a file at `MODEL_PATH` when that env var is set.
    pub fn load_default() -> Result<Self> {
        match std::env::var("MODEL_PATH") {
            Ok(p) if !p.trim().is_empty() => Self::load(Path::new(p.trim())),
            _ => Self::from_bytes(EMBEDDED_MODEL).context("parsing embedded model"),
        }
    }

    /// Load a model from a file path.
    pub fn load(path: &Path) -> Result<Self> {
        let buf = std::fs::read(path)
            .with_context(|| format!("reading model file {}", path.display()))?;
        Self::from_bytes(&buf).with_context(|| format!("parsing model {}", path.display()))
    }

    /// Spam probability in `[0, 1]` for `text`.
    pub fn score(&self, text: &str) -> f32 {
        let mut s = self.bias as f64;
        for b in features::buckets(
            text,
            self.num_buckets,
            self.char_min,
            self.char_max,
            self.word_ngram_max,
        ) {
            s += self.weights[b as usize] as f64;
        }
        (1.0 / (1.0 + (-s).exp())) as f32
    }

    /// Returns `(is_spam, probability)` using `threshold_override` or the
    /// threshold baked into the model.
    pub fn is_spam(&self, text: &str, threshold_override: Option<f32>) -> (bool, f32) {
        let p = self.score(text);
        let thr = threshold_override.unwrap_or(self.threshold);
        (p >= thr, p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_model_loads() {
        let m = Model::load_default().expect("embedded model must load");
        assert!(m.num_buckets > 0);
        assert!(m.threshold > 0.0 && m.threshold < 1.0);
    }

    #[test]
    fn rejects_bad_magic() {
        assert!(Model::from_bytes(b"NOPE....................").is_err());
    }
}
