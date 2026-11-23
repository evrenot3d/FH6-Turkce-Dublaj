//! Cümle seslerini parçanın zamanına yerleştirme ve arka planla karıştırma.
//! Her cümle kendi İngilizce başlangıcında başlar (önceki bitmediyse ondan sonra); bir sonraki konuşmaya kadar
//! sığmazsa perde bozulmadan (ffmpeg atempo) en çok `en_cok_hiz` kat hızlandırılır.

use std::path::Path;

use anyhow::Result;

use crate::ses;

pub const SR: u32 = 44100;

pub struct Yer {
    pub start: f64,
    pub end: f64,
    /// İngilizce aslının ses gücü (Türkçe ses buna eşlenir; bağırma yüksek, fısıltı alçak kalır)
    pub rms: f32,
}

/// → (Türkçe konuşma izi, her cümlenin [başlangıç, bitiş] ve hız katı)
pub fn yerlestir(sesler: &[Option<Vec<f32>>], yerler: &[Yer], sure: f64, en_cok_hiz: f32, gecici: &Path) -> Result<(Vec<f32>, Vec<Option<(f64, f64, f32)>>)> {
    let mut iz = vec![0f32; ((sure + 0.05) * SR as f64) as usize];
    let mut imlec = 0.0f64;
    let mut bilgi = Vec::new();
    for (i, y) in yerler.iter().enumerate() {
        let Some(a) = sesler.get(i).cloned().flatten().filter(|a| !a.is_empty()) else {
            bilgi.push(None);
            continue;
        };
        let mut a = a;
        let ro = ses::etkin_rms(&a, SR);
        if y.rms > 0.0 && ro > 0.0 {
            let k = (y.rms / ro).clamp(0.25, 4.0);
            a.iter_mut().for_each(|v| *v *= k);
        }
        // sonraki cümle (kim söylerse söylesin) ya da parça sonu
        let sonraki = yerler[i + 1..].iter().map(|x| x.start).find(|&s| s > y.start).unwrap_or(sure);
        let bas = y.start.max(imlec);
        let yer = (sonraki - bas - 0.05).max(0.4);
        let mut f = 1.0f32;
        if a.len() as f64 / SR as f64 > yer * 1.03 {
            f = ((a.len() as f64 / SR as f64 / yer) as f32).min(en_cok_hiz);
            if f > 1.02 {
                a = ses::atempo(&a, SR, f, gecici)?;
            }
        }
        let d = a.len() as f64 / SR as f64;
        let i0 = (bas * SR as f64) as usize;
        if i0 + a.len() > iz.len() {
            iz.resize(i0 + a.len(), 0.0);
        }
        // 5 ms yumuşak giriş / çıkış (tık sesi olmasın)
        let g = (0.005 * SR as f64) as usize;
        let n = a.len();
        for (k, v) in a.iter().enumerate() {
            let w = (k.min(n - 1 - k) as f32 / g as f32).min(1.0);
            iz[i0 + k] += v * w;
        }
        bilgi.push(Some((bas, bas + d, f)));
        imlec = bas + d + 0.05;
    }
    Ok((iz, bilgi))
}

/// arka plan (kanal kanal) + Türkçe konuşma → çıkış (kanal kanal), tepe sınırlı
pub fn karistir(arka: &[Vec<f32>], dub: &[f32], arka_seviye: f32, dub_seviye: f32) -> Vec<Vec<f32>> {
    let n = arka.first().map_or(dub.len(), |k| k.len());
    let mut out: Vec<Vec<f32>> = arka
        .iter()
        .map(|k| (0..n).map(|i| k[i] * arka_seviye + dub.get(i).copied().unwrap_or(0.0) * dub_seviye).collect())
        .collect();
    if out.is_empty() {
        out = vec![dub.iter().map(|v| v * dub_seviye).collect()];
    }
    // yumuşak sınırlayıcı: tepe 0,98'i aşarsa bütün parça kısılır
    let tepe = out.iter().flat_map(|k| k.iter()).fold(0.0f32, |m, v| m.max(v.abs()));
    if tepe > 0.98 {
        let k = 0.98 / tepe;
        out.iter_mut().for_each(|c| c.iter_mut().for_each(|v| *v *= k));
    }
    out
}

/// WAV → Opus (eklentiye gönderilen küçük dosya)
pub fn opus(wav: &Path, cikis: &Path) -> Result<()> {
    ses::ffmpeg(&["-i", &wav.to_string_lossy(), "-c:a", "libopus", "-b:a", "128k", "-vbr", "on", &cikis.to_string_lossy()])
}
