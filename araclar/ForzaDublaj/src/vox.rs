//! VoxCPM2 (OpenBMB; GGUF Q8_0, audio.cpp-gguf deposundan) — audio.cpp sunucusuyla (`bin/audiocpp/audiocpp_server.exe`,
//! CUDA; Evrenot'taki RoFormer sunucusunun aynısı, başka port). Ses rengi her cümlede aynı referans WAV'dan
//! (voice_ref, metinsiz: yalnız konuşmacı koşullaması, İngilizce metin verilmez → aksan taşımaz) ve aynı tohumla.
//! Adım sayısı (num_inference_steps) ayarlanabilir; varsayılan 10, burada 30.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use base64::Engine;
use serde_json::{Value, json};

use crate::surec::{Surec, ajan, yanit_al, yol_str};
use crate::yollar::Yollar;

pub const PORT: u16 = 17895;
pub const MODEL: &str = "voxcpm2/voxcpm2-q8_0.gguf";

pub struct Vox {
    surec: Surec,
    ayar: PathBuf,
    pub model: PathBuf,
    /// Linux (bulut sunucusu): audio.cpp yerine resmî VoxCPM2 Python paketi, aynı istek biçimiyle (linux/vox_sunucu.py)
    python: Option<PathBuf>,
}

impl Vox {
    pub fn new(y: &Yollar) -> Self {
        let betik = y.kok.join("linux").join("vox_sunucu.py");
        if !cfg!(windows) {
            return Vox {
                surec: Surec::new("VoxCPM2", PathBuf::from("/usr/bin/env"), y.kok.join("voxcpm2.log"), format!("http://127.0.0.1:{PORT}/health")),
                ayar: y.calisma.join("voxcpm2_ayar.json"),
                model: betik.clone(),
                python: Some(betik),
            };
        }
        let exe = [y.bin.join("audiocpp"), PathBuf::from(crate::yollar::EVRENOT).join("bin").join("audiocpp")]
            .into_iter()
            .map(|d| d.join("audiocpp_server.exe"))
            .find(|p| p.is_file())
            .unwrap_or_default();
        let model = [y.kok.join("modeller").join(MODEL), y.modeller.join(MODEL)].into_iter().find(|p| p.is_file()).unwrap_or_else(|| y.kok.join("modeller").join(MODEL));
        Vox {
            surec: Surec::new("VoxCPM2", exe, y.kok.join("voxcpm2.log"), format!("http://127.0.0.1:{PORT}/health")),
            ayar: y.calisma.join("voxcpm2_ayar.json"),
            model,
            python: None,
        }
    }

    pub fn baslat(&mut self) -> Result<()> {
        if self.surec.calisiyor() {
            return Ok(());
        }
        if let Some(betik) = &self.python {
            let args: Vec<OsString> = vec!["python3".into(), betik.clone().into_os_string(), "--port".into(), PORT.to_string().into()];
            return self.surec.baslat(&args, "vox", Duration::from_secs(900));
        }
        if !self.model.is_file() {
            bail!("VoxCPM2 modeli yok: {} (voxcpm2_indir.bat'ı çalıştırın)", self.model.display());
        }
        let ayar = json!({
            "host": "127.0.0.1", "port": PORT, "backend": "cuda", "device": 0, "threads": 4,
            "lazy_load": true, "idle_unload_ms": 0, "min_free_memory_mb": 0,
            "max_request_body_bytes": 400_000_000u64,
            "models": [
                {"id": "vox", "family": "voxcpm2", "path": yol_str(&self.model), "task": "tts", "mode": "offline"}
            ]
        });
        std::fs::write(&self.ayar, serde_json::to_string_pretty(&ayar)?)?;
        let args: Vec<OsString> = vec!["--config".into(), self.ayar.clone().into_os_string()];
        self.surec.baslat(&args, "vox", Duration::from_secs(300))
    }

    pub fn durdur(&mut self) {
        self.surec.durdur();
    }
}

/// emotion2vec+ etiketi → VoxCPM2 söyleyiş talimatı (metnin başına parantez içinde; okunmaz, yalnız tonu yönlendirir)
pub fn talimat(duygu: &str) -> Option<&'static str> {
    Some(match duygu {
        "kizgin" => "angry, intense tone",
        "uzgun" => "sad, soft tone",
        "mutlu" => "happy, cheerful tone",
        "korkmus" => "scared, tense tone",
        "igrenmis" => "disgusted tone",
        "saskin" => "surprised, excited tone",
        _ => return None,
    })
}

/// Bir cümle, referans sesin rengiyle, Türkçe → (mono ses, örnekleme hızı). `duygu`: emotion2vec etiketi (belirginse)
pub fn oku(metin: &str, referans: &Path, tohum: u64, adim: u32, duygu: Option<&str>) -> Result<(Vec<f32>, u32)> {
    let metin = match duygu.and_then(talimat) {
        Some(t) => format!("({t}){metin}"),
        None => metin.to_string(),
    };
    let govde = json!({
        "model": "vox",
        "request": {
            "text": metin,
            "language": "tr",
            "seed": tohum % 4_294_967_295,
            "voice_ref": yol_str(referans),
            "options": {"num_inference_steps": adim}
        }
    });
    let r = ajan(Duration::from_secs(900)).post(&format!("http://127.0.0.1:{PORT}/v1/tasks/run")).send_json(&govde)?;
    let b = yanit_al("VoxCPM2", r)?;
    let v: Value = serde_json::from_slice(&b).context("VoxCPM2 yanıtı okunamadı")?;
    let ses = v["audio"]
        .as_str()
        .or_else(|| v["named_audio_outputs"].as_array().and_then(|a| a.first()).and_then(|o| o["audio"].as_str()))
        .context("VoxCPM2 ses döndürmedi")?;
    let veri = base64::engine::general_purpose::STANDARD.decode(ses.trim_start_matches("data:audio/wav;base64,"))?;
    let s = crate::ses::wav_oku_bayt(&veri)?;
    if s.uzunluk() == 0 {
        bail!("VoxCPM2 boş ses döndü");
    }
    Ok((s.mono(), s.sr))
}
