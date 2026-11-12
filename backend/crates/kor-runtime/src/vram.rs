//! §19.3: a local model larger than the GPU's memory runs partly on the CPU, many times slower.
//! When a session starts on a local server, the model's size (as the server reports it) is
//! compared with the largest GPU in the system profile, and the user is told before the first
//! answer crawls. Nothing is said when either figure is unknown.

use kor_i18n::{Localizer, args};
use kor_providers::Provider;

/// Room for the context cache and runtime buffers on top of the weights.
const HEADROOM: f64 = 1.1;

/// The note for a model of `model_bytes` on a GPU with `vram_mb` (largest adapter), if it does
/// not fit.
pub fn note(l: &Localizer, model: &str, model_bytes: u64, vram_mb: u64) -> Option<String> {
    #[allow(clippy::cast_precision_loss)] // sizes in bytes, shown to one decimal
    let (need_gb, have_gb, size_gb) = (
        model_bytes as f64 * HEADROOM / 1e9,
        vram_mb as f64 * 1_048_576.0 / 1e9,
        model_bytes as f64 / 1e9,
    );
    (need_gb > have_gb).then(|| {
        l.t_args(
            "vram-short",
            &args!(
                "model" => model.to_owned(),
                "size" => format!("{size_gb:.1}"),
                "vram" => format!("{have_gb:.1}")
            ),
        )
    })
}

/// The check for the session's model: only for local servers, and only when the server reports
/// the size and the system profile knows the GPU.
pub async fn check(l: &Localizer, provider: &dyn Provider, model: &str) -> Option<String> {
    if !crate::is_local_provider(provider.manifest()) {
        return None;
    }
    let vram = crate::kor_home()
        .and_then(|h| kor_health::system::load(&h))
        .and_then(|p| p.gpus.iter().filter_map(|g| g.vram_mb).max())?;
    let models = tokio::time::timeout(std::time::Duration::from_secs(3), provider.list_models())
        .await
        .ok()?
        .ok()?;
    let size = models.iter().find(|m| m.id == model)?.size_bytes?;
    note(l, model, size, vram)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warns_only_when_the_weights_do_not_fit() {
        let l = Localizer::new(kor_i18n::Lang::En);
        // 9 GB of weights on an 8 GB card: warned, with both sizes.
        let n = note(&l, "qwen3:14b", 9_000_000_000, 8 * 1024).expect("warned");
        assert!(
            n.contains("qwen3:14b") && n.contains("9.0") && n.contains("8.6"),
            "{n}"
        );
        // 4 GB on 12 GB: fine.
        assert_eq!(note(&l, "llama3.2", 4_000_000_000, 12 * 1024), None);
    }
}
