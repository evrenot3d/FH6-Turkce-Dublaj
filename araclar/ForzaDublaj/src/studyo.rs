//! Ses işleme (stüdyo miksi) — eski `ses_isleme.py`'nin karşılığı. Yapay zekâ değil, dublaj stüdyolarının klasik son
//! işleme adımları; işlenmemiş dublaj saklanır, bunlar onun üzerine uygulanır (anahtar değişince yeniden okunmaz).
//!   mixm  ses rengi eşleme (dublajın frekans dağılımı orijinal konuşmaya, ±6 dB, yarı yarıya) + hafif oda yankısı
//!   mixd  80 Hz altı kesme, de-esser, 3,5 kHz yumuşatma, kompresör (ffmpeg)
//!   mixl  algılanan ses yüksekliği (LUFS) orijinal konuşmayla eşlenir, tepeler −1 dB'de sınırlanır
//!   mixs  cümle cümle mesafe ve ortam (her Türkçe cümle kendi İngilizce cümlesinin tınısına / uzaklığına göre)
//!   mixt  hafif analog tını (asimetrik yumuşak doygunluk)
//!   mixr  oda sesi (orijinalin sessiz anlarındaki ortam uğultusu, çok düşük seviyede)
//!   mixg  Türkçe konuşurken arka plan yumuşakça alçalır (ducking) + karışımda hafif ortak kompresör

use std::path::Path;

use anyhow::Result;
use realfft::RealFftPlanner;
use realfft::num_complex::Complex;

use crate::ses;

pub const SR: u32 = 48000;
pub const ANAHTARLAR: [(&str, &str); 7] = [
    ("mixm", "Ses rengi eşleme + ortam yankısı"),
    ("mixd", "De-esser, EQ ve kompresör"),
    ("mixl", "Loudness standardı"),
    ("mixs", "Cümle cümle mesafe ve ortam"),
    ("mixt", "Hafif tını rengi"),
    ("mixr", "Oda sesi"),
    ("mixg", "Yapıştırıcı + ducking"),
];
const DINAMIK: &str = "highpass=f=80,deesser=i=0.4:m=0.5:f=0.5:s=o,equalizer=f=3500:t=q:w=1.2:g=-1.5,acompressor=threshold=0.1:ratio=3:attack=8:release=120:makeup=1";
const SINIRLAYICI: &str = "alimiter=limit=0.891:attack=5:release=60:level=disabled";

// ------------------------------------------------------------------ süzgeçler
/// İkinci dereceden IIR süzgeç (scipy lfilter, a0 = 1)
fn iir(x: &[f64], b: [f64; 3], a: [f64; 3]) -> Vec<f64> {
    let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
    x.iter()
        .map(|&v| {
            let y = b[0] * v + b[1] * x1 + b[2] * x2 - a[1] * y1 - a[2] * y2;
            (x2, x1, y2, y1) = (x1, v, y1, y);
            y
        })
        .collect()
}

/// Butterworth 2. derece alçak geçiren (scipy butter ile aynı: ön bükmeli çift doğrusal dönüşüm)
fn alcak_gecir2(x: &[f64], fc: f64, sr: f64) -> Vec<f64> {
    let k = (std::f64::consts::PI * fc / sr).tan();
    let r2 = std::f64::consts::SQRT_2;
    let n = 1.0 / (1.0 + r2 * k + k * k);
    let b0 = k * k * n;
    iir(x, [b0, 2.0 * b0, b0], [1.0, 2.0 * (k * k - 1.0) * n, (1.0 - r2 * k + k * k) * n])
}

/// Butterworth 1. derece yüksek geçiren
fn yuksek_gecir1(x: &[f64], fc: f64, sr: f64) -> Vec<f64> {
    let k = (std::f64::consts::PI * fc / sr).tan();
    let b0 = 1.0 / (1.0 + k);
    iir(x, [b0, -b0, 0.0], [1.0, (k - 1.0) / (k + 1.0), 0.0])
}

/// FFT ile evrişim (tam uzunluk: x + h − 1), örtüşmeli toplama — uzun seste bellek sabit kalır
fn evrisim(x: &[f32], h: &[f64]) -> Vec<f32> {
    if x.is_empty() || h.is_empty() {
        return Vec::new();
    }
    let n_fft = (h.len() * 4).next_power_of_two().max(4096);
    let blok = n_fft - h.len() + 1;
    let mut p = RealFftPlanner::<f64>::new();
    let (ileri, geri) = (p.plan_fft_forward(n_fft), p.plan_fft_inverse(n_fft));
    let mut hb = vec![0f64; n_fft];
    hb[..h.len()].copy_from_slice(h);
    let mut hs = ileri.make_output_vec();
    ileri.process(&mut hb, &mut hs).expect("fft");
    let mut out = vec![0f32; x.len() + h.len() - 1];
    let mut giris = ileri.make_input_vec();
    let mut sp = ileri.make_output_vec();
    let mut y = geri.make_output_vec();
    for s in (0..x.len()).step_by(blok) {
        let e = (s + blok).min(x.len());
        giris.iter_mut().for_each(|v| *v = 0.0);
        for (i, v) in x[s..e].iter().enumerate() {
            giris[i] = *v as f64;
        }
        ileri.process(&mut giris, &mut sp).expect("fft");
        for (a, b) in sp.iter_mut().zip(&hs) {
            *a *= b;
        }
        geri.process(&mut sp, &mut y).expect("ifft");
        for i in 0..(e - s + h.len() - 1).min(out.len() - s) {
            out[s + i] += (y[i] / n_fft as f64) as f32;
        }
    }
    out
}

/// scipy fftconvolve(mode="same")
fn evrisim_ayni(x: &[f32], h: &[f64]) -> Vec<f32> {
    let tam = evrisim(x, h);
    let bas = (h.len() - 1) / 2;
    tam[bas..bas + x.len()].to_vec()
}

/// scipy welch (Hann, %50 örtüşme, ortalama çıkarma, yoğunluk ölçeği) → (frekans, güç)
fn welch(x: &[f32], sr: f64, nps: usize) -> (Vec<f64>, Vec<f64>) {
    let nps = nps.min(x.len());
    let w: Vec<f64> = (0..nps).map(|n| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * n as f64 / nps as f64).cos()).collect();
    let olcek = 1.0 / (sr * w.iter().map(|v| v * v).sum::<f64>());
    let fft = RealFftPlanner::<f64>::new().plan_fft_forward(nps);
    let mut g = fft.make_input_vec();
    let mut c = fft.make_output_vec();
    let nf = nps / 2 + 1;
    let mut p = vec![0f64; nf];
    let mut k = 0;
    let mut s = 0;
    while s + nps <= x.len() {
        let ort = x[s..s + nps].iter().map(|&v| v as f64).sum::<f64>() / nps as f64;
        for i in 0..nps {
            g[i] = (x[s + i] as f64 - ort) * w[i];
        }
        fft.process(&mut g, &mut c).expect("fft");
        for (i, v) in c.iter().enumerate() {
            let carp = if i == 0 || (nps % 2 == 0 && i == nf - 1) { 1.0 } else { 2.0 };
            p[i] += v.norm_sqr() * olcek * carp;
        }
        k += 1;
        s += nps / 2;
    }
    p.iter_mut().for_each(|v| *v /= k.max(1) as f64);
    ((0..nf).map(|i| i as f64 * sr / nps as f64).collect(), p)
}

/// scipy firwin2(1025, frekans (0..1), kazanç), Hamming penceresi
fn firwin2(n: usize, frek: &[f64], kazanc: &[f64]) -> Vec<f64> {
    let nf = 1 + (n as f64).log2().ceil().exp2() as usize;
    let fx: Vec<Complex<f64>> = (0..nf)
        .map(|i| {
            let x = i as f64 / (nf - 1) as f64;
            let j = frek.partition_point(|&f| f < x).clamp(1, frek.len() - 1);
            let (f0, f1) = (frek[j - 1], frek[j]);
            let t = if f1 > f0 { ((x - f0) / (f1 - f0)).clamp(0.0, 1.0) } else { 0.0 };
            let g = kazanc[j - 1] + t * (kazanc[j] - kazanc[j - 1]);
            let ph = -((n - 1) as f64 / 2.0) * std::f64::consts::PI * x;
            Complex::new(g * ph.cos(), g * ph.sin())
        })
        .collect();
    let m = 2 * (nf - 1);
    let fft = RealFftPlanner::<f64>::new().plan_fft_inverse(m);
    let mut g = fx;
    g[0].im = 0.0;
    g[nf - 1].im = 0.0;
    let mut y = fft.make_output_vec();
    fft.process(&mut g, &mut y).expect("ifft");
    (0..n).map(|i| y[i] / m as f64 * (0.54 - 0.46 * (2.0 * std::f64::consts::PI * i as f64 / (n - 1) as f64).cos())).collect()
}

/// Basit rastgele sayı üreteci (yankı gürültüsü için; tekrarlanabilir)
struct Rastgele(u64);

impl Rastgele {
    fn tek(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }

    fn normal(&mut self) -> f64 {
        let (u, v) = (self.tek().max(1e-12), self.tek());
        (-2.0 * u.ln()).sqrt() * (2.0 * std::f64::consts::PI * v).cos()
    }
}

// ------------------------------------------------------------------ ölçüm
/// ITU-R BS.1770-4 bütünleşik ses yüksekliği (LUFS)
pub fn lufs(x: &[f32]) -> f64 {
    if x.len() < (SR / 2) as usize {
        return -70.0;
    }
    let d: Vec<f64> = x.iter().map(|&v| v as f64).collect();
    let d = iir(&d, [1.53512485958697, -2.69169618940638, 1.19839281085285], [1.0, -1.69065929318241, 0.73248077421585]);
    let d = iir(&d, [1.0, -2.0, 1.0], [1.0, -1.99004745483398, 0.99007225036621]);
    let (blok, adim) = ((0.4 * SR as f64) as usize, (0.1 * SR as f64) as usize);
    if d.len() < blok {
        return -70.0;
    }
    let n = 1 + (d.len() - blok) / adim;
    let ms: Vec<f64> = (0..n).map(|i| d[i * adim..i * adim + blok].iter().map(|v| v * v).sum::<f64>() / blok as f64).collect();
    let lk = |m: f64| -0.691 + 10.0 * (m + 1e-12).log10();
    let g: Vec<f64> = ms.iter().copied().filter(|&m| lk(m) > -70.0).collect();
    if g.is_empty() {
        return -70.0;
    }
    let rel = -0.691 + 10.0 * (g.iter().sum::<f64>() / g.len() as f64).log10() - 10.0;
    let g: Vec<f64> = ms.iter().copied().filter(|&m| lk(m) > rel.max(-70.0)).collect();
    if g.is_empty() { -70.0 } else { lk(g.iter().sum::<f64>() / g.len() as f64) }
}

// ------------------------------------------------------------------ adımlar
/// Ses rengi eşleme: dublajın uzun süreli frekans dağılımı orijinale (100 Hz – 12 kHz, ±6 dB, yarı yarıya)
fn renk_esle(dub: &[f32], ref_: &[f32], miktar: f64, en_az_sn: f64) -> Vec<f32> {
    let sr = SR as f64;
    if (ref_.len() as f64) < sr * en_az_sn || (dub.len() as f64) < sr * en_az_sn {
        return dub.to_vec();
    }
    let nps = if ref_.len().min(dub.len()) >= SR as usize { 4096 } else { 1024 };
    let (f, pd) = welch(dub, sr, nps);
    let (_, pr) = welch(ref_, sr, nps);
    let bant: Vec<bool> = f.iter().map(|&x| (100.0..=12000.0).contains(&x)).collect();
    let mut fark: Vec<f64> = pr.iter().zip(&pd).map(|(r, d)| 10.0 * ((r + 1e-14) / (d + 1e-14)).log10()).collect();
    let (mut sw, mut s) = (0.0, 0.0);
    for i in 0..f.len() {
        if bant[i] {
            sw += pd[i] + 1e-14;
            s += fark[i] * (pd[i] + 1e-14);
        }
    }
    let ort = s / sw;
    fark.iter_mut().for_each(|v| *v -= ort); // yalnız biçim
    let yum: Vec<f64> = (0..f.len())
        .map(|i| {
            if f[i] <= 0.0 {
                return fark[i];
            }
            let (lo, hi) = (f[i] / 2f64.powf(1.0 / 6.0), f[i] * 2f64.powf(1.0 / 6.0));
            let v: Vec<f64> = (0..f.len()).filter(|&k| f[k] >= lo && f[k] <= hi).map(|k| fark[k]).collect();
            v.iter().sum::<f64>() / v.len().max(1) as f64
        })
        .collect();
    let kaz: Vec<f64> = (0..f.len()).map(|i| if bant[i] { 10f64.powf((yum[i] * miktar).clamp(-6.0, 6.0) / 20.0) } else { 1.0 }).collect();
    let frek: Vec<f64> = f.iter().map(|x| x / (sr / 2.0)).collect();
    evrisim_ayni(dub, &firwin2(1025, &frek, &kaz))
}

/// Kısa oda yankısı (rt60 sn, ıslak oran); kuyruk=true ise yankı kuyruğu da döner
fn oda(dub: &[f32], rt60: f64, islak: f64, tohum: u64, kuyruk: bool) -> Vec<f32> {
    let sr = SR as f64;
    let n = (sr * rt60 * 1.2) as usize;
    let mut r = Rastgele(tohum.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let ir: Vec<f64> = (0..n).map(|i| r.normal() * (-6.91 * i as f64 / sr / rt60).exp()).collect();
    let ir = alcak_gecir2(&ir, 6000.0, sr);
    let mut h = vec![0f64; (0.012 * sr) as usize]; // 12 ms ön gecikme
    h.extend(ir);
    let e = h.iter().map(|v| v * v).sum::<f64>().sqrt() + 1e-12;
    h.iter_mut().for_each(|v| *v /= e);
    let yank = evrisim(dub, &h);
    if kuyruk {
        let mut out: Vec<f32> = yank.iter().map(|v| (islak * *v as f64) as f32).collect();
        for (i, v) in dub.iter().enumerate() {
            out[i] += v;
        }
        out
    } else {
        dub.iter().zip(&yank).map(|(d, y)| d + (islak * *y as f64) as f32).collect()
    }
}

/// İngilizce cümlenin uzaklık ipuçları: tiz oranı (dB), zarf dalgalanması (dB std), seviye (dB)
fn uzaklik_ipucu(x: &[f32]) -> Option<[f64; 3]> {
    if x.len() < (0.3 * SR as f64) as usize {
        return None;
    }
    let (f, p) = welch(x, SR as f64, 1024);
    let top = |a: f64, b: f64, ust: bool| f.iter().zip(&p).filter(|(fr, _)| **fr >= a && if ust { **fr <= b } else { **fr < b }).map(|(_, v)| v).sum::<f64>();
    let tiz = 10.0 * ((top(3000.0, 8000.0, true) + 1e-14) / (top(200.0, 3000.0, false) + 1e-14)).log10();
    let adim = (0.01 * SR as f64) as usize;
    let db: Vec<f64> = (0..x.len().saturating_sub(adim))
        .step_by(adim)
        .map(|i| 20.0 * ((x[i..i + adim].iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / adim as f64 + 1e-12).sqrt()).log10())
        .collect();
    let tepe = db.iter().cloned().fold(f64::MIN, f64::max);
    let etkin: Vec<f64> = db.into_iter().filter(|&v| v > tepe - 40.0).collect();
    let dalga = if etkin.len() > 5 {
        let m = etkin.iter().sum::<f64>() / etkin.len() as f64;
        (etkin.iter().map(|v| (v - m).powi(2)).sum::<f64>() / etkin.len() as f64).sqrt()
    } else {
        6.0
    };
    let sev = 20.0 * ((x.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / x.len() as f64).sqrt() + 1e-9).log10();
    Some([tiz, dalga, sev])
}

/// Cümle cümle mesafe ve ortam: her Türkçe cümle (zaman) kendi İngilizce cümlesine (cumle) göre tını + yankı alır
fn sahne_esle(dub: &[f32], ref_: &[f32], cumle: &[(f64, f64)], zaman: &[Option<[f64; 2]>]) -> Vec<f32> {
    let sr = SR as f64;
    let mut ciftler: Vec<(f64, f64, f64, f64)> = cumle.iter().zip(zaman).filter_map(|(c, t)| t.map(|t| (t[0], t[1], c.0, c.1))).collect();
    if ciftler.is_empty() {
        return oda(&renk_esle(dub, ref_, 0.5, 1.0), 0.3, 0.12, 7, false);
    }
    ciftler.sort_by(|a, b| a.0.total_cmp(&b.0));
    let kes = |a: f64, b: f64| &ref_[((a * sr) as usize).min(ref_.len())..((b * sr) as usize).min(ref_.len())];
    let ipucu: Vec<Option<[f64; 3]>> = ciftler.iter().map(|c| uzaklik_ipucu(kes(c.2, c.3))).collect();
    let tamam: Vec<[f64; 3]> = ipucu.iter().flatten().copied().collect();
    let (mut ort, mut sd) = ([0.0; 3], [1e-6; 3]);
    if tamam.len() >= 3 {
        for k in 0..3 {
            ort[k] = tamam.iter().map(|v| v[k]).sum::<f64>() / tamam.len() as f64;
            sd[k] = (tamam.iter().map(|v| (v[k] - ort[k]).powi(2)).sum::<f64>() / tamam.len() as f64).sqrt() + 1e-6;
        }
    }
    let uzak: Vec<f64> = ipucu
        .iter()
        .map(|f| match f {
            Some(f) if tamam.len() >= 3 => {
                let z: Vec<f64> = (0..3).map(|k| (f[k] - ort[k]) / sd[k]).collect();
                (0.35 + 0.22 * -(0.25 * z[0] + 0.5 * z[1] + 0.25 * z[2])).clamp(0.0, 1.0)
            }
            _ => 0.35,
        })
        .collect();
    let n = dub.len();
    let baslar: Vec<usize> = ciftler.iter().map(|c| ((c.0 - 0.03).max(0.0) * sr) as usize).collect();
    let mut sinir = vec![0];
    sinir.extend_from_slice(&baslar[1..]);
    sinir.push(n);
    let mut out = vec![0f32; n + SR as usize];
    for j in 0..ciftler.len() {
        let (b0, b1) = (sinir[j].min(n), sinir[j + 1].min(n));
        if b1 <= b0 {
            continue;
        }
        let d = uzak[j];
        let parca = renk_esle(&dub[b0..b1], kes(ciftler[j].2, ciftler[j].3), 0.5, 0.4);
        let parca = oda(&parca, 0.22 + 0.38 * d, 0.05 + 0.2 * d, 7 + j as u64, true);
        for (i, v) in parca.iter().enumerate() {
            if b0 + i < out.len() {
                out[b0 + i] += v;
            }
        }
    }
    out.truncate(n);
    out
}

/// Hafif analog tını: asimetrik yumuşak doygunluk (çift harmonik), sonra 20 Hz yüksek geçiren (doğru akım)
fn tini(x: &[f32]) -> Vec<f32> {
    let tepe = x.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-9) as f64;
    let (surme, kayma, karisim) = (1.8f64, 0.08f64, 0.25f64);
    let y: Vec<f64> = x
        .iter()
        .map(|&v| {
            let u = v as f64 / tepe;
            let doy = ((surme * (u + kayma)).tanh() - (surme * kayma).tanh()) / surme.tanh();
            (1.0 - karisim) * u + karisim * doy
        })
        .collect();
    yuksek_gecir1(&y, 20.0, SR as f64).into_iter().map(|v| (v * tepe) as f32).collect()
}

/// Oda sesi: orijinalin sessiz anlarının tınısında, o seviyede (en çok −48 dB) gürültü; bulunamazsa None
fn oda_sesi(ref_: &[f32], n: usize) -> Option<Vec<f32>> {
    let adim = (0.05 * SR as f64) as usize;
    if ref_.len() < 20 * adim {
        return None;
    }
    let db: Vec<f64> = (0..ref_.len() - adim)
        .step_by(adim)
        .map(|i| 20.0 * ((ref_[i..i + adim].iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / adim as f64).sqrt() + 1e-9).log10())
        .collect();
    let mut canli: Vec<f64> = db.iter().copied().filter(|&v| v > -90.0).collect();
    if canli.len() < 10 {
        return None;
    }
    canli.sort_by(|a, b| a.total_cmp(b));
    let yuzde = |p: f64| {
        let k = p / 100.0 * (canli.len() - 1) as f64;
        let (a, b) = (k.floor() as usize, k.ceil() as usize);
        canli[a] + (canli[b] - canli[a]) * (k - a as f64)
    };
    let (lo, hi) = (yuzde(5.0), yuzde(25.0));
    if hi > yuzde(90.0) - 15.0 {
        return None; // ayrı sessiz an yok
    }
    let idx: Vec<usize> = (0..db.len()).filter(|&i| db[i] >= lo && db[i] <= hi).collect();
    if idx.len() < 6 {
        return None;
    }
    let sessiz: Vec<f32> = idx.iter().flat_map(|&i| ref_[i * adim..(i + 1) * adim].iter().copied()).collect();
    let sev = (sessiz.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / sessiz.len() as f64).sqrt();
    if 20.0 * (sev + 1e-9).log10() < -72.0 {
        return None;
    }
    let st = crate::stft::Stft::new(2048, 512, false);
    let (t, sp) = st.ileri(&sessiz);
    let nf = st.frekans();
    let buyukluk: Vec<f32> = (0..nf).map(|f| ((0..t).map(|k| sp[(f * t + k) * 2].powi(2) + sp[(f * t + k) * 2 + 1].powi(2)).sum::<f32>() / t as f32).sqrt()).collect();
    let kare = n / 512 + 4;
    let mut r = Rastgele(0x5EED_0011);
    let mut gs = vec![0f32; nf * kare * 2];
    for f in 0..nf {
        for k in 0..kare {
            let ph = 2.0 * std::f64::consts::PI * r.tek();
            gs[(f * kare + k) * 2] = buyukluk[f] * ph.cos() as f32;
            gs[(f * kare + k) * 2 + 1] = buyukluk[f] * ph.sin() as f32;
        }
    }
    let mut y = st.geri(&gs, kare, n);
    let rms = (y.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / y.len().max(1) as f64).sqrt() + 1e-12;
    let hedef = (sev * 0.8).min(10f64.powf(-48.0 / 20.0));
    y.iter_mut().for_each(|v| *v = (*v as f64 * hedef / rms) as f32);
    Some(y)
}

/// Türkçe konuşurken arka plan yumuşakça alçalır: derinlik 1,5–5 dB (arka plan konuşmaya yakınsa çok), iniş 80 ms,
/// çıkış 450 ms, cümle başından 60 ms önce. `arka`: kanal kanal arka plan (arka_sr), `dub`: 48 kHz mono
pub fn arka_plani_alcalt(arka: &[Vec<f32>], arka_sr: u32, dub: &[f32]) -> Vec<Vec<f32>> {
    let adim = (0.01 * SR as f64) as usize;
    let zarf: Vec<f64> = (0..dub.len().saturating_sub(adim).max(1))
        .step_by(adim)
        .map(|i| (dub[i..(i + adim).min(dub.len())].iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / adim as f64 + 1e-12).sqrt())
        .collect();
    let edb: Vec<f64> = zarf.iter().map(|v| 20.0 * v.log10()).collect();
    let esik = (edb.iter().cloned().fold(f64::MIN, f64::max) - 35.0).max(-50.0);
    let mut etkin: Vec<f64> = edb.iter().map(|&v| if v > esik { 1.0 } else { 0.0 }).collect();
    let on: Vec<f64> = (0..etkin.len()).map(|i| etkin.get(i + 6).copied().unwrap_or(0.0)).collect();
    for (a, b) in etkin.iter_mut().zip(&on) {
        *a = a.max(*b);
    }
    let mono: Vec<f32> = (0..arka[0].len()).map(|i| arka.iter().map(|k| k[i]).sum::<f32>() / arka.len() as f32).collect();
    let ba = (0.01 * arka_sr as f64) as usize;
    let bz: Vec<f64> = (0..mono.len().saturating_sub(ba).max(1))
        .step_by(ba)
        .map(|i| (mono[i..(i + ba).min(mono.len())].iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / ba as f64 + 1e-12).sqrt())
        .collect();
    let m = bz.len().min(etkin.len());
    if m == 0 || etkin[..m].iter().sum::<f64>() < 5.0 {
        return arka.to_vec();
    }
    let ac: Vec<usize> = (0..m).filter(|&i| etkin[i] > 0.0).collect();
    let rms = |v: &[f64]| (ac.iter().map(|&i| v[i] * v[i]).sum::<f64>() / ac.len() as f64).sqrt() + 1e-9;
    let goreli = 20.0 * (rms(&bz) / rms(&zarf)).log10();
    let derinlik = (1.5 + (goreli + 12.0) * 0.25).clamp(1.5, 5.0);
    let (yukari, asagi) = (1.0 - (-1.0f64 / 8.0).exp(), 1.0 - (-1.0f64 / 45.0).exp());
    let mut cur = 0.0;
    let kazanc: Vec<f64> = etkin
        .iter()
        .map(|&v| {
            cur += (v - cur) * if v > cur { yukari } else { asagi };
            10f64.powf(-derinlik * cur / 20.0)
        })
        .collect();
    arka.iter()
        .map(|k| {
            k.iter()
                .enumerate()
                .map(|(i, &v)| {
                    let t = i as f64 / arka_sr as f64 / 0.01;
                    let j = t.floor() as usize;
                    let g = if j + 1 < kazanc.len() { kazanc[j] + (kazanc[j + 1] - kazanc[j]) * (t - j as f64) } else { 1.0 };
                    (v as f64 * g) as f32
                })
                .collect()
        })
        .collect()
}

fn ffmpeg_suzgec(x: &[f32], af: &str, gecici: &Path) -> Result<Vec<f32>> {
    std::fs::create_dir_all(gecici)?;
    let (g, c) = (gecici.join("_mx_g.wav"), gecici.join("_mx_c.wav"));
    ses::wav_yaz_mono(&g, x, SR)?;
    ses::ffmpeg(&["-i", &g.to_string_lossy(), "-af", af, "-ar", &SR.to_string(), "-ac", "1", &c.to_string_lossy()])?;
    let y = ses::wav_oku(&c)?;
    let _ = std::fs::remove_file(&g);
    let _ = std::fs::remove_file(&c);
    Ok(ses::yeniden_ornekle(&y.mono(), y.sr, SR))
}

/// İşlenmemiş dublaj (48 kHz mono) + orijinal konuşma → işlenmiş dublaj.
/// `cumle`: İngilizce cümlelerin, `zaman`: Türkçe karşılıklarının parça içi zamanları (cümle cümle eşleme için)
pub fn isle(dub: &[f32], konusma: &[f32], anahtar: &[String], gecici: &Path, cumle: &[(f64, f64)], zaman: &[Option<[f64; 2]>]) -> Result<Vec<f32>> {
    let var = |k: &str| anahtar.iter().any(|a| a == k);
    let mut x = dub.to_vec();
    if x.iter().all(|v| *v == 0.0) {
        return Ok(x);
    }
    let l0 = lufs(&x);
    if var("mixs") && !cumle.is_empty() && !zaman.is_empty() {
        x = sahne_esle(&x, konusma, cumle, zaman);
    } else if var("mixm") {
        x = oda(&renk_esle(&x, konusma, 0.5, 1.0), 0.3, 0.12, 7, false);
    }
    if var("mixd") {
        x = ffmpeg_suzgec(&x, DINAMIK, gecici)?;
    }
    if var("mixt") {
        x = tini(&x);
    }
    if var("mixr") {
        if let Some(r) = oda_sesi(konusma, x.len()) {
            for (a, b) in x.iter_mut().zip(&r) {
                *a += b;
            }
        }
    }
    let carp = |x: &mut Vec<f32>, db: f64| {
        let k = 10f64.powf(db.clamp(-12.0, 12.0) / 20.0) as f32;
        x.iter_mut().for_each(|v| *v *= k);
    };
    if var("mixl") {
        let hedef = lufs(konusma);
        let hedef = if hedef > -60.0 { hedef } else { l0 };
        let simdi = lufs(&x);
        carp(&mut x, hedef - simdi);
        x = ffmpeg_suzgec(&x, SINIRLAYICI, gecici)?;
    } else {
        let simdi = lufs(&x); // işleme ses seviyesini değiştirmesin
        if simdi > -60.0 && l0 > -60.0 {
            carp(&mut x, l0 - simdi);
        }
    }
    let tepe = x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    if tepe > 0.98 {
        x.iter_mut().for_each(|v| *v *= 0.98 / tepe);
    }
    Ok(x)
}
