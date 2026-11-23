//! Kim konuşuyor: kadın / erkek (voice-gender-classifier, ONNX q8) ve ses izi (WeSpeaker ResNet34-LM, ONNX).
//! SlateVox V2'den alındı (hesaplar aynı). İkisi de işlemcide çalışır (ekran kartını meşgul etmez).

use std::path::Path;

use anyhow::Result;
use ort::session::Session;
use ort::value::Tensor;
use realfft::RealFftPlanner;

const SR: usize = 16000;

// ------------------------------------------------------------------ kadın / erkek
pub struct Cinsiyet {
    oturum: Session,
}

impl Cinsiyet {
    pub fn ac(model: &Path) -> Result<Self> {
        Ok(Cinsiyet { oturum: crate::onnx::oturum(model, false)?.0 })
    }

    /// Kadın sesi olasılığı (16 kHz ses parçası).
    pub fn kadin(&mut self, x: &[f32]) -> Result<f32> {
        let r = self.oturum.run(ort::inputs!["input_values" => Tensor::from_array(([1usize, x.len()], x.to_vec()))?])?;
        let (_, l) = r["logits"].try_extract_tensor::<f32>()?;
        let m = l[0].max(l[1]);
        let (e0, e1) = ((l[0] - m).exp(), (l[1] - m).exp());
        Ok(e1 / (e0 + e1))
    }

    /// Uzun seste 4 sn'lik pencerelerin ortalaması (en çok 4 pencere).
    pub fn kadin_uzun(&mut self, x: &[f32]) -> Result<f32> {
        let w = 4 * SR;
        if x.len() <= w * 3 / 2 {
            return self.kadin(x);
        }
        let mut t = 0.0;
        let mut n = 0;
        let mut s = 0;
        while s + SR < x.len() && n < 4 {
            t += self.kadin(&x[s..(s + w).min(x.len())])?;
            n += 1;
            s += w;
        }
        Ok(t / n as f32)
    }
}

// ------------------------------------------------------------------ ses izi
pub struct SesIzi {
    oturum: Session,
    mel: Vec<f64>,
}

impl SesIzi {
    pub fn ac(model: &Path) -> Result<Self> {
        Ok(SesIzi { oturum: crate::onnx::oturum(model, false)?.0, mel: kaldi_mel() })
    }

    /// 256 boyutlu, birim uzunlukta ses izi (16 kHz ses).
    pub fn iz(&mut self, x16: &[f32]) -> Result<Vec<f32>> {
        let (n, f) = kaldi_fbank(x16, &self.mel);
        let r = self.oturum.run(ort::inputs!["feats" => Tensor::from_array(([1usize, n, 80], f))?])?;
        let (_, e) = r["embs"].try_extract_tensor::<f32>()?;
        let norm = e.iter().map(|v| v * v).sum::<f32>().sqrt() + 1e-9;
        Ok(e.iter().map(|v| v / norm).collect())
    }
}

fn kaldi_mel() -> Vec<f64> {
    let (n_mel, nfft) = (80, 512);
    let mel = |f: f64| 1127.0 * (1.0 + f / 700.0).ln();
    let (lo, hi) = (mel(20.0), mel(8000.0));
    let d = (hi - lo) / (n_mel + 1) as f64;
    let mut bank = vec![0f64; n_mel * (nfft / 2 + 1)];
    for b in 0..n_mel {
        let (l, c, r) = (lo + b as f64 * d, lo + (b + 1) as f64 * d, lo + (b + 2) as f64 * d);
        for k in 0..nfft / 2 {
            let fm = mel(16000.0 / nfft as f64 * k as f64);
            bank[b * (nfft / 2 + 1) + k] = ((fm - l) / (c - l)).min((r - fm) / (r - c)).max(0.0);
        }
    }
    bank
}

/// torchaudio.compliance.kaldi.fbank ile aynı (25/10 ms, Hamming, ön vurgu 0,97, dither yok, 80 mel) + ortalama çıkarma
fn kaldi_fbank(x16: &[f32], mel: &[f64]) -> (usize, Vec<f32>) {
    let (l, s, nfft) = (400usize, 160usize, 512usize);
    let mut x: Vec<f64> = x16.iter().map(|&v| v as f64 * 32768.0).collect();
    if x.len() < l {
        x.resize(l, 0.0);
    }
    let n = 1 + (x.len() - l) / s;
    let win: Vec<f64> = (0..l).map(|i| 0.54 - 0.46 * (2.0 * std::f64::consts::PI * i as f64 / (l - 1) as f64).cos()).collect();
    let fft = RealFftPlanner::<f64>::new().plan_fft_forward(nfft);
    let mut giris = fft.make_input_vec();
    let mut cikis = fft.make_output_vec();
    let nb = nfft / 2 + 1;
    let mut m = vec![0f64; n * 80];
    for k in 0..n {
        let fr = &x[k * s..k * s + l];
        let ort = fr.iter().sum::<f64>() / l as f64;
        let d: Vec<f64> = fr.iter().map(|v| v - ort).collect();
        giris.iter_mut().for_each(|v| *v = 0.0);
        for i in 0..l {
            let ov = if i == 0 { d[0] * 0.03 } else { d[i] - 0.97 * d[i - 1] };
            giris[i] = ov * win[i];
        }
        fft.process(&mut giris, &mut cikis).expect("fft");
        for b in 0..80 {
            let e: f64 = (0..nb).map(|f| mel[b * nb + f] * cikis[f].norm_sqr()).sum();
            m[k * 80 + b] = e.max(f32::EPSILON as f64).ln();
        }
    }
    for b in 0..80 {
        let ort = (0..n).map(|k| m[k * 80 + b]).sum::<f64>() / n as f64;
        for k in 0..n {
            m[k * 80 + b] -= ort;
        }
    }
    (n, m.into_iter().map(|v| v as f32).collect())
}
