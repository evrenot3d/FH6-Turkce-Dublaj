//! ZONOS3-Mini (Zyphra, Apache-2.0): kendi ggml sunucusu (`bin/zonos3/zonos3-server.exe`, kaynaktan CUDA 13.0 ile
//! derlendi, ekran kartında; CUDA DLL'leri yanında). Modeller `modeller/zonos3/` (backbone-ptq, zac-keep2 kod çözücü, spk-encoder, steer duygu yönleri).
//! Ses rengi 192 boyutlu konuşmacı izinden (embedding kipi): aksan taşımadan yalnız tını. Duygu: steer bundle'daki
//! `emotions.*` yönleri (tını değişmez, yalnız söyleyiş).

use std::ffi::OsString;
use std::time::Duration;

use anyhow::{Result, bail};
use base64::Engine;
use serde_json::json;

use crate::surec::{Surec, ajan, yanit_al};
use crate::yollar::Yollar;

pub const PORT: u16 = 17894;

pub struct Zonos {
    surec: Surec,
    backbone: std::path::PathBuf,
}

impl Zonos {
    pub fn new(y: &Yollar) -> Self {
        Zonos {
            surec: Surec::new(
                "ZONOS3",
                y.bin.join("zonos3").join(crate::yollar::exe("zonos3-server")),
                y.kok.join("zonos3.log"),
                format!("http://127.0.0.1:{PORT}/v2/capabilities"),
            ),
            backbone: y.modeller.join("zonos3").join("backbone-ptq.gguf"),
        }
    }

    pub fn hazir_mi(&self) -> bool {
        self.surec.var_mi() && self.backbone.is_file()
    }

    pub fn baslat(&mut self) -> Result<()> {
        if self.surec.calisiyor() {
            return Ok(());
        }
        let args: Vec<OsString> = vec![
            self.backbone.clone().into_os_string(),
            "--host".into(),
            "127.0.0.1".into(),
            "--port".into(),
            PORT.to_string().into(),
            "--gpu".into(),
        ];
        self.surec.baslat(&args, "zonos", Duration::from_secs(180))
    }

    pub fn durdur(&mut self) {
        self.surec.durdur();
    }
}

/// emotion2vec+ etiketi → ZONOS3 duygu yönü
fn duygu_yonu(duygu: &str) -> Option<&'static str> {
    Some(match duygu {
        "kizgin" => "emotions.angry",
        "uzgun" => "emotions.sad",
        "mutlu" => "emotions.happy",
        "korkmus" => "emotions.fearful",
        "igrenmis" => "emotions.disgusted",
        "saskin" => "emotions.surprised",
        _ => return None,
    })
}

/// Bir sesin 192 boyutlu ZONOS3 ses izi (sunucunun konuşmacı kodlayıcısıyla). Her cümlede ses örneğini yeniden
/// işletmek yerine bir kez hesaplanır: seslendirme 5,5 kat hızlanır (1 dk konuşma 34,6 → 6,2 sn, ölçüldü).
pub fn ses_izi(wav: &[u8]) -> Result<Vec<f32>> {
    let a = ajan(Duration::from_secs(120));
    let r = a
        .post(&format!("http://127.0.0.1:{PORT}/v2/speakers"))
        .header("X-TTS-Session-ID", "ytdublaj")
        .send_json(json!({"speaker_audio_base64": base64::engine::general_purpose::STANDARD.encode(wav), "label": "kisi", "speaker_audio_name": "kisi.wav"}))?;
    let v: serde_json::Value = serde_json::from_slice(&yanit_al("ZONOS3 ses izi", r)?)?;
    let id = v["id"].as_str().unwrap_or_default().to_string();
    let r = a.get(&format!("http://127.0.0.1:{PORT}/v2/speakers/{id}/embedding")).header("X-TTS-Session-ID", "ytdublaj").call()?;
    let v: serde_json::Value = serde_json::from_slice(&yanit_al("ZONOS3 ses izi", r)?)?;
    let e: Vec<f32> = v["embedding"].as_array().map(|x| x.iter().filter_map(|y| y.as_f64()).map(|y| y as f32).collect()).unwrap_or_default();
    if e.len() != 192 {
        bail!("ZONOS3 ses izi okunamadı");
    }
    Ok(e)
}

/// İki ses izi arasında küresel geçiş (slerp): t=0 a, t=1 b. Uzunluk da doğrusal geçer.
pub fn karistir(a: &[f32], b: &[f32], t: f32) -> Vec<f32> {
    let na = a.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
    let nb = b.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
    let ua: Vec<f32> = a.iter().map(|x| x / na).collect();
    let ub: Vec<f32> = b.iter().map(|x| x / nb).collect();
    let c = ua.iter().zip(&ub).map(|(x, y)| x * y).sum::<f32>().clamp(-1.0, 1.0);
    let om = c.acos();
    let (ka, kb) = if om < 1e-4 { (1.0 - t, t) } else { (((1.0 - t) * om).sin() / om.sin(), (t * om).sin() / om.sin()) };
    let n = (1.0 - t) * na + t * nb;
    ua.iter().zip(&ub).map(|(x, y)| (ka * x + kb * y) * n).collect()
}

/// float32 dizisi → .npy baytları (sunucu ses izini bu biçimde alır)
fn npy(v: &[f32]) -> Vec<u8> {
    let mut h = format!("{{'descr': '<f4', 'fortran_order': False, 'shape': ({},), }}", v.len());
    while (10 + h.len() + 1) % 64 != 0 {
        h.push(' ');
    }
    h.push('\n');
    let mut b = vec![0x93u8, b'N', b'U', b'M', b'P', b'Y', 1, 0];
    b.extend_from_slice(&(h.len() as u16).to_le_bytes());
    b.extend_from_slice(h.as_bytes());
    for x in v {
        b.extend_from_slice(&x.to_le_bytes());
    }
    b
}

/// Bir cümle, kişinin ses iziyle, Türkçe. → (mono ses, 44,1 kHz)
pub fn oku(metin: &str, iz: Option<&[f32]>, tohum: u64, sure_sn: f64, duygu: Option<(&str, f32)>) -> Result<(Vec<f32>, u32)> {
    let mut govde = json!({
        "text": metin, "language": "tr", "stream": false, "format": "wav",
        "seed": tohum % 4_294_967_295, "max_tokens": ((sure_sn * 15.0 * 2.2) as usize + 45).min(2000),
    });
    if let Some(e) = iz {
        govde["speaker_embedding_base64"] = json!(base64::engine::general_purpose::STANDARD.encode(npy(e)));
        govde["speaker_mode"] = json!("embedding");
    }
    if let Some((d, guc)) = duygu
        && let Some(y) = duygu_yonu(d)
    {
        govde["controls"] = json!({ y: guc });
    }
    let r = ajan(Duration::from_secs(300)).post(&format!("http://127.0.0.1:{PORT}/v2/generate")).send_json(&govde)?;
    let b = yanit_al("ZONOS3", r)?;
    let s = crate::ses::wav_oku_bayt(&b)?;
    if s.uzunluk() == 0 {
        bail!("ZONOS3 boş ses döndü");
    }
    Ok((s.mono(), s.sr))
}
