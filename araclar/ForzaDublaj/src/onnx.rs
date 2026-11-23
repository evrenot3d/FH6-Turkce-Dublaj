//! ONNX Runtime: `bin/ort/onnxruntime.dll` (CUDA'lı) program açılırken bir kez yüklenir. CUDA DLL'leri aynı klasörde.
//! Oturum açılırken ekran kartı (CUDA) denenir, olmazsa işlemci kullanılır.

use std::path::Path;
use std::sync::OnceLock;

use anyhow::{Context, Result};
use ort::environment::Environment;
use ort::session::Session;

static ORTAM: OnceLock<Environment> = OnceLock::new();

/// ONNX Runtime'ı yükler (yalnız ilk çağrıda iş yapar).
pub fn hazirla(bin: &Path) -> Result<()> {
    if ORTAM.get().is_some() {
        return Ok(());
    }
    let dir = bin.join("ort");
    // CUDA sağlayıcısı cuDNN / cuBLAS DLL'lerini PATH'te arar
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut yeni = std::ffi::OsString::from(dir.as_os_str());
    yeni.push(if cfg!(windows) { ";" } else { ":" });
    yeni.push(path);
    // SAFETY: program açılışında, başka iş parçacıkları ortam değişkeni okumadan önce çağrılır
    unsafe { std::env::set_var("PATH", yeni) };
    let kutuphane = if cfg!(windows) { "onnxruntime.dll" } else { "libonnxruntime.so" };
    let env = ort::init_from(dir.join(kutuphane))
        .with_context(|| format!("{kutuphane} yüklenemedi (bin/ort)"))?
        .with_name("evrenot")
        .build()?;
    let _ = ORTAM.set(env);
    Ok(())
}

/// Model oturumu: önce ekran kartı, olmazsa işlemci. (oturum, ekran kartında mı)
pub fn oturum(model: &Path, ekran_karti: bool) -> Result<(Session, bool)> {
    let env = ORTAM.get().context("ONNX Runtime hazırlanmadı")?;
    if ekran_karti {
        let b = Session::builder(env)?
            .with_execution_providers([ort::ep::CUDA::default()
                .with_arena_extend_strategy(ort::ep::ArenaExtendStrategy::SameAsRequested)
                .build()
                .error_on_failure()]);
        if let Ok(mut b) = b {
            if let Ok(s) = b.commit_from_file(model) {
                return Ok((s, true));
            }
        }
        eprintln!("[onnx] ekran kartı kullanılamadı, işlemci: {}", model.display());
    }
    let mut b = Session::builder(env)?;
    Ok((b.commit_from_file(model).with_context(|| format!("model açılamadı: {}", model.display()))?, false))
}
