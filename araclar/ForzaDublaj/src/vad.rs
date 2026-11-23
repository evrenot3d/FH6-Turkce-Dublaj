//! Konuşma algılama: Silero VAD v6 (ONNX, işlemcide; `modeller/silero_vad_v6.onnx`). faster-whisper'ın
//! `get_speech_timestamps` hesabının aynısı (eski sürüm Whisper'ı bununla çalıştırıyordu: en az 500 ms sessizlik).

use std::path::Path;

use anyhow::Result;
use ort::session::Session;
use ort::value::Tensor;

const PENCERE: usize = 512; // 16 kHz'de 32 ms
const BAGLAM: usize = 64;

pub struct Ayarlar {
    pub esik: f32,
    pub en_az_sessizlik_ms: f32,
    pub pay_ms: f32,
}

impl Default for Ayarlar {
    fn default() -> Self {
        Ayarlar { esik: 0.5, en_az_sessizlik_ms: 500.0, pay_ms: 400.0 }
    }
}

pub struct Vad {
    oturum: Session,
}

impl Vad {
    pub fn ac(model: &Path) -> Result<Self> {
        Ok(Vad { oturum: crate::onnx::oturum(model, false)?.0 })
    }

    /// Her 32 ms'lik pencerenin konuşma olasılığı.
    fn olasiliklar(&mut self, x16: &[f32]) -> Result<Vec<f32>> {
        let n = x16.len().div_ceil(PENCERE);
        let mut dolu = x16.to_vec();
        dolu.resize(n * PENCERE, 0.0);
        // her pencerenin önüne bir önceki pencerenin son 64 örneği (ilk pencerede sıfır)
        let mut giris = Vec::with_capacity(n * (PENCERE + BAGLAM));
        for i in 0..n {
            if i == 0 {
                giris.extend(std::iter::repeat_n(0.0, BAGLAM));
            } else {
                giris.extend_from_slice(&dolu[i * PENCERE - BAGLAM..i * PENCERE]);
            }
            giris.extend_from_slice(&dolu[i * PENCERE..(i + 1) * PENCERE]);
        }
        let mut h = vec![0f32; 128];
        let mut c = vec![0f32; 128];
        let mut out = Vec::with_capacity(n);
        for blok in giris.chunks((PENCERE + BAGLAM) * 10000) {
            let b = blok.len() / (PENCERE + BAGLAM);
            let r = self.oturum.run(ort::inputs![
                "input" => Tensor::from_array(([b, PENCERE + BAGLAM], blok.to_vec()))?,
                "h" => Tensor::from_array(([1usize, 1, 128], h.clone()))?,
                "c" => Tensor::from_array(([1usize, 1, 128], c.clone()))?,
            ])?;
            out.extend_from_slice(r["speech_probs"].try_extract_tensor::<f32>()?.1);
            h = r["hn"].try_extract_tensor::<f32>()?.1.to_vec();
            c = r["cn"].try_extract_tensor::<f32>()?.1.to_vec();
        }
        out.truncate(n);
        Ok(out)
    }

    /// Konuşma bölümleri (örnek sırası, 16 kHz): [(başlangıç, bitiş)]
    pub fn bolumler(&mut self, x16: &[f32], a: &Ayarlar) -> Result<Vec<(usize, usize)>> {
        let p = self.olasiliklar(x16)?;
        let sr = 16000.0;
        let neg = (a.esik - 0.15).max(0.01);
        let en_az_sessizlik = (sr * a.en_az_sessizlik_ms / 1000.0) as usize;
        let pay = (sr * a.pay_ms / 1000.0) as usize;
        let uzunluk = x16.len();
        let mut konusmalar: Vec<(usize, usize)> = Vec::new();
        let (mut tetik, mut bas, mut gecici_son) = (false, 0usize, 0usize);
        for (i, &v) in p.iter().enumerate() {
            let s = PENCERE * i;
            if v >= a.esik && gecici_son != 0 {
                gecici_son = 0;
            }
            if v >= a.esik && !tetik {
                tetik = true;
                bas = s;
                continue;
            }
            if v < neg && tetik {
                if gecici_son == 0 {
                    gecici_son = s;
                }
                if s - gecici_son < en_az_sessizlik {
                    continue;
                }
                konusmalar.push((bas, gecici_son));
                gecici_son = 0;
                tetik = false;
            }
        }
        if tetik && uzunluk > bas {
            konusmalar.push((bas, uzunluk));
        }
        // her yana pay; aradaki sessizlik iki paydan kısaysa ortadan bölüşülür
        let n = konusmalar.len();
        for i in 0..n {
            if i == 0 {
                konusmalar[i].0 = konusmalar[i].0.saturating_sub(pay);
            }
            if i + 1 < n {
                let ara = konusmalar[i + 1].0.saturating_sub(konusmalar[i].1);
                if ara < 2 * pay {
                    konusmalar[i].1 += ara / 2;
                    konusmalar[i + 1].0 = konusmalar[i + 1].0.saturating_sub(ara / 2);
                } else {
                    konusmalar[i].1 = (konusmalar[i].1 + pay).min(uzunluk);
                    konusmalar[i + 1].0 = konusmalar[i + 1].0.saturating_sub(pay);
                }
            } else {
                konusmalar[i].1 = (konusmalar[i].1 + pay).min(uzunluk);
            }
        }
        Ok(konusmalar)
    }
}
