//! Fish Audio Drama 3 (35B MoE, 3B etkin; açık ağırlık) — vLLM-Omni sunucusu (`vllm serve fishaudio/drama-3 --omni`),
//! OpenAI uyumlu /v1/audio/speech. Ses rengi karakterin sabit referans repliğinden (ses + Whisper metni), sabit tohumla;
//! duygu, metnin başına köşeli parantez içinde oyunculuk yönü olarak ("[cheerful, upbeat] Merhaba!").
//! Yalnız bulut sunucusunda (linux/v2).

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use base64::Engine;
use serde_json::json;

use crate::surec::{ajan, yanit_al, yol_str};

pub const PORT: u16 = 17896;

/// Sunucunun kabul ettiği istek biçimi (0: referans data-URL + tohum, 1: data-URL, 2: file:// + tohum, 3: file://);
/// ilk çalışan hatırlanır
static BICIM: AtomicUsize = AtomicUsize::new(0);

/// Bir cümle, referans sesin rengiyle → (mono ses, örnekleme hızı). `yon`: İngilizce oyunculuk yönü (Qwen3-Omni'den)
pub fn oku(metin: &str, referans: &Path, ref_metin: &str, tohum: u64, yon: Option<&str>) -> Result<(Vec<f32>, u32)> {
    let girdi = match yon.map(str::trim) {
        Some(t) if !t.is_empty() => format!("[{t}] {metin}"),
        _ => metin.to_string(),
    };
    let veri = std::fs::read(referans).with_context(|| format!("{} okunamadı", referans.display()))?;
    let data_url = format!("data:audio/wav;base64,{}", base64::engine::general_purpose::STANDARD.encode(&veri));
    let dosya_url = format!("file://{}", yol_str(referans));
    let mut son_hata = String::new();
    for b in BICIM.load(Ordering::Relaxed)..4 {
        let mut govde = json!({
            "model": "fishaudio/drama-3",
            "input": girdi,
            "voice": "default",
            "ref_text": ref_metin,
            "response_format": "wav",
            "temperature": 0.8,
            "top_p": 0.9,
            "top_k": 30
        });
        govde["ref_audio"] = json!(if b < 2 { data_url.as_str() } else { dosya_url.as_str() });
        if b % 2 == 0 {
            govde["seed"] = json!(tohum % 2_147_483_647);
        }
        let r = ajan(Duration::from_secs(600)).post(&format!("http://127.0.0.1:{PORT}/v1/audio/speech")).send_json(&govde)?;
        let kod = r.status().as_u16();
        match yanit_al("Drama 3", r) {
            Ok(bayt) => {
                BICIM.store(b, Ordering::Relaxed);
                let s = crate::ses::wav_oku_bayt(&bayt)?;
                if s.uzunluk() == 0 {
                    bail!("Drama 3 boş ses döndü");
                }
                return Ok((s.mono(), s.sr));
            }
            Err(e) if (400..500).contains(&kod) => {
                son_hata = format!("{e:#}");
            }
            Err(e) => return Err(e),
        }
    }
    bail!("Drama 3 isteği kabul etmedi: {son_hata}")
}
