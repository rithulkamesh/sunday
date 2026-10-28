//! Embeddings that run inside the app, with no Ollama: all-MiniLM-L6-v2 (384 numbers per text) on the CPU, through
//! candle. The model is a few files under `~/.cache/sunday/models/embed/all-MiniLM-L6-v2`, put there by
//! `scripts/fetch-embed-model.sh`; the app never downloads. Used for memory search when Settings picks it.
use crate::brain::llm::UserMsg;
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config};
use std::sync::Mutex;
use tokenizers::{PaddingParams, Tokenizer, TruncationParams};

type Res<T> = Result<T, Box<dyn std::error::Error>>;

pub const NAME: &str = "all-MiniLM-L6-v2";
const MAX_TOKENS: usize = 256;

struct Loaded {
    model: BertModel,
    tokenizer: Tokenizer,
}

static LOADED: Mutex<Option<Loaded>> = Mutex::new(None);

fn dir() -> std::path::PathBuf {
    crate::voice::models::dir().join("embed").join(NAME)
}

pub fn installed() -> bool {
    ["config.json", "tokenizer.json", "model.safetensors"].iter().all(|f| dir().join(f).exists())
}

fn load() -> Res<Loaded> {
    if !installed() {
        return Err(UserMsg(format!("The on-device embedding model isn't downloaded. Run scripts/fetch-embed-model.sh.")).into());
    }
    let d = dir();
    let config: Config = serde_json::from_str(&std::fs::read_to_string(d.join("config.json"))?)?;
    let vb = VarBuilder::from_buffered_safetensors(std::fs::read(d.join("model.safetensors"))?, DType::F32, &Device::Cpu)?;
    let mut tokenizer = Tokenizer::from_file(d.join("tokenizer.json")).map_err(|e| e.to_string())?;
    tokenizer
        .with_padding(Some(PaddingParams::default()))
        .with_truncation(Some(TruncationParams { max_length: MAX_TOKENS, ..Default::default() }))
        .map_err(|e| e.to_string())?;
    Ok(Loaded { model: BertModel::load(vb, &config)?, tokenizer })
}

/// One unit-length vector per text: the mean of the token vectors, as the model was trained to be used.
pub fn embed(texts: &[String]) -> Res<Vec<Vec<f32>>> {
    let mut g = LOADED.lock().unwrap();
    if g.is_none() {
        *g = Some(load()?);
    }
    let Loaded { model, tokenizer } = g.as_ref().unwrap();
    let enc = tokenizer.encode_batch(texts.to_vec(), true).map_err(|e| e.to_string())?;
    let dev = Device::Cpu;
    let rows = |f: &dyn Fn(&tokenizers::Encoding) -> Vec<u32>| -> Res<Tensor> {
        let n = enc[0].len();
        let flat: Vec<u32> = enc.iter().flat_map(|e| f(e)).collect();
        Ok(Tensor::from_vec(flat, (enc.len(), n), &dev)?)
    };
    let ids = rows(&|e| e.get_ids().to_vec())?;
    let mask = rows(&|e| e.get_attention_mask().to_vec())?;
    let types = ids.zeros_like()?;
    let hidden = model.forward(&ids, &types, Some(&mask))?; // [batch, tokens, 384]
    let m = mask.to_dtype(DType::F32)?.unsqueeze(2)?;
    let mean = hidden.broadcast_mul(&m)?.sum(1)?.broadcast_div(&m.sum(1)?)?;
    let unit = mean.broadcast_div(&mean.sqr()?.sum_keepdim(1)?.sqrt()?)?;
    Ok(unit.to_vec2::<f32>()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs the model files (scripts/fetch-embed-model.sh); skipped without them.
    #[test]
    #[ignore = "needs the on-device embedding model"]
    fn similar_texts_are_closer_and_vectors_are_unit_length() {
        let v = embed(&["my driving test is in February".into(), "when do I take my driving exam".into(), "TCP slow start".into()]).unwrap();
        assert!(v.iter().all(|e| e.len() == 384 && (e.iter().map(|x| x * x).sum::<f32>() - 1.0).abs() < 1e-3));
        let dot = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
        assert!(dot(&v[0], &v[1]) > dot(&v[0], &v[2]) + 0.2);
    }
}
