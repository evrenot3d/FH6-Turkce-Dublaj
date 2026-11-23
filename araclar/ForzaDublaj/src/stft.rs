//! PyTorch'un `torch.stft` / `torch.istft` hesabının aynısı (center=True, reflect dolgu, periyodik Hann penceresi,
//! tek taraflı spektrum, isteğe bağlı "normalized": pencere enerjisinin köküne bölme). BandIt v2 bunu bekliyor.
//! Spektrum düzeni: [frekans][kare][gerçek, sanal] tek düz dizide (ONNX girişi (…, F, T, 2) ile aynı sıra).

use std::sync::Arc;

use realfft::num_complex::Complex32;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};

pub struct Stft {
    pub n_fft: usize,
    pub hop: usize,
    pencere: Vec<f32>,
    olcek: f32,
    ileri: Arc<dyn RealToComplex<f32>>,
    geri: Arc<dyn ComplexToReal<f32>>,
}

impl Stft {
    pub fn new(n_fft: usize, hop: usize, normalized: bool) -> Self {
        let pencere: Vec<f32> =
            (0..n_fft).map(|n| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * n as f64 / n_fft as f64).cos() as f32).collect();
        let olcek = if normalized { pencere.iter().map(|w| w * w).sum::<f32>().sqrt() } else { 1.0 };
        let mut p = RealFftPlanner::<f32>::new();
        Stft { n_fft, hop, ileri: p.plan_fft_forward(n_fft), geri: p.plan_fft_inverse(n_fft), pencere, olcek }
    }

    pub fn frekans(&self) -> usize {
        self.n_fft / 2 + 1
    }

    pub fn kare_sayisi(&self, uzunluk: usize) -> usize {
        1 + uzunluk / self.hop
    }

    /// x → (kare sayısı, spektrum [F][T][2])
    pub fn ileri(&self, x: &[f32]) -> (usize, Vec<f32>) {
        let p = self.n_fft / 2;
        // reflect dolgu: x[p], …, x[1] | x | x[n-2], …, x[n-1-p]
        let n = x.len();
        let mut d = Vec::with_capacity(n + 2 * p);
        d.extend((1..=p).rev().map(|i| x[i.min(n - 1)]));
        d.extend_from_slice(x);
        d.extend((1..=p).map(|i| x[n.saturating_sub(1 + i)]));
        let t = self.kare_sayisi(n);
        let f = self.frekans();
        // kareler işlemci çekirdeklerine paylaştırılır; her kare kendi FFT tamponuyla
        use rayon::prelude::*;
        let kareler: Vec<Vec<Complex32>> = (0..t)
            .into_par_iter()
            .map_init(
                || (self.ileri.make_input_vec(), self.ileri.make_output_vec()),
                |(giris, cikis), k| {
                    let s = k * self.hop;
                    for (i, g) in giris.iter_mut().enumerate() {
                        *g = d[s + i] * self.pencere[i];
                    }
                    self.ileri.process(giris, cikis).expect("fft");
                    cikis.clone()
                },
            )
            .collect();
        let mut out = vec![0f32; f * t * 2];
        for (k, c) in kareler.iter().enumerate() {
            for (fi, v) in c.iter().enumerate() {
                let o = (fi * t + k) * 2;
                out[o] = v.re / self.olcek;
                out[o + 1] = v.im / self.olcek;
            }
        }
        (t, out)
    }

    /// spektrum [F][T][2] → ses (uzunluk örnek)
    pub fn geri(&self, spek: &[f32], t: usize, uzunluk: usize) -> Vec<f32> {
        let f = self.frekans();
        let p = self.n_fft / 2;
        let toplam = self.n_fft + self.hop * (t - 1);
        let mut y = vec![0f32; toplam];
        let mut zarf = vec![0f32; toplam];
        let w2: Vec<f32> = self.pencere.iter().map(|w| w * w).collect();
        use rayon::prelude::*;
        let kareler: Vec<Vec<f32>> = (0..t)
            .into_par_iter()
            .map_init(
                || (self.geri.make_input_vec(), self.geri.make_output_vec()),
                |(giris, cikis), k| {
                    for (fi, g) in giris.iter_mut().enumerate() {
                        let o = (fi * t + k) * 2;
                        *g = Complex32::new(spek[o] * self.olcek, spek[o + 1] * self.olcek);
                    }
                    // tek taraflı spektrumun ilk ve son kutusu gerçek olmalı (irfft sanal kısmı yok sayar)
                    giris[0].im = 0.0;
                    giris[f - 1].im = 0.0;
                    self.geri.process(giris, cikis).expect("ifft");
                    cikis.iter().zip(&self.pencere).map(|(v, w)| v / self.n_fft as f32 * w).collect()
                },
            )
            .collect();
        for (k, kare) in kareler.iter().enumerate() {
            let s = k * self.hop;
            for i in 0..self.n_fft {
                y[s + i] += kare[i];
                zarf[s + i] += w2[i];
            }
        }
        (0..uzunluk)
            .map(|i| {
                let j = i + p;
                if j < toplam && zarf[j] > 1e-11 { y[j] / zarf[j] } else { 0.0 }
            })
            .collect()
    }
}

#[cfg(test)]
mod testler {
    use super::*;

    #[test]
    fn ileri_geri_ayni_ses() {
        let s = Stft::new(2048, 512, true);
        let x: Vec<f32> = (0..48000).map(|i| ((i as f32 * 0.013).sin() + (i as f32 * 0.0021).cos()) * 0.3).collect();
        let (t, spek) = s.ileri(&x);
        assert_eq!(t, 1 + 48000 / 512);
        let y = s.geri(&spek, t, x.len());
        let hata = x.iter().zip(&y).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(hata < 1e-4, "hata {hata}");
    }
}
