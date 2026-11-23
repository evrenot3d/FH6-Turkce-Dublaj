//! İngilizce konuşma → kelimeler (başlangıç, bitiş, metin). Whisper large-v3-turbo, ONNX (onnx-community
//! `whisper-large-v3-turbo_timestamped`, fp16; `modeller/whisper-onnx`).
//!
//! Akış (eski sürümdeki faster-whisper ayarlarının karşılığı: İngilizce, konuşma algılamalı, önceki metne bağlamadan):
//!   1. Silero VAD konuşma bölümlerini bulur (en az 500 ms sessizlik, 400 ms pay).
//!   2. Bölümler ≤ 30 sn'lik gruplara toplanır; grup içi bölümler art arda eklenir (sessizlik atılır).
//!   3. Her grup: log-mel → kodlayıcı → kod çözücü (açgözlü, zaman damgasız metin).
//!   4. Kelime zamanları: kod çözücünün çapraz dikkatinden (hizalama başlıkları) DTW ile — OpenAI Whisper'ın
//!      `find_alignment` yöntemi. Zamanlar grubun içinden videodaki gerçek zamana çevrilir.

use std::path::Path;

use anyhow::{Context, Result};
use ort::session::Session;
use ort::session::SessionInputValue;
use ort::value::{DynValue, Tensor};
use tokenizers::Tokenizer;

use crate::stft::Stft;

const SR: usize = 16000;
const N_MEL: usize = 128;
const KARE: usize = 3000; // 30 sn
const ORNEK_30: usize = 480_000;
const KATMAN: usize = 4;
const BASLIK: usize = 20;
const BOYUT: usize = 64;

const EOT: i64 = 50257;
const SOT: i64 = 50258;
const EN: i64 = 50259;
const TRANSCRIBE: i64 = 50360;
const NO_TS: i64 = 50364;
const EN_COK_TOKEN: usize = 440;

/// Kelime: (başlangıç sn, bitiş sn, metin — başında boşluk olabilir, Whisper'daki gibi)
pub type Kelime = (f64, f64, String);

pub struct Whisper {
    kodlayici: Session,
    cozucu: Session,
    tok: Tokenizer,
    mel: Vec<f32>,
    stft: Stft,
    bastirilan: Vec<usize>,
    hizalama: Vec<(usize, usize)>,
    pub ekran_karti: bool,
}

/// librosa.filters.mel(sr=16000, n_fft=400, n_mels=128) — Slaney ölçeği ve normu (Whisper'ın filtreleri)
fn mel_filtreleri() -> Vec<f32> {
    let n_fft = 400;
    let nf = n_fft / 2 + 1;
    let hz_mel = |f: f64| {
        let (f_sp, min_log_hz) = (200.0 / 3.0, 1000.0);
        let min_log_mel = min_log_hz / f_sp;
        let logstep = (6.4f64).ln() / 27.0;
        if f >= min_log_hz { min_log_mel + (f / min_log_hz).ln() / logstep } else { f / f_sp }
    };
    let mel_hz = |m: f64| {
        let (f_sp, min_log_hz) = (200.0 / 3.0, 1000.0);
        let min_log_mel = min_log_hz / f_sp;
        let logstep = (6.4f64).ln() / 27.0;
        if m >= min_log_mel { min_log_hz * (logstep * (m - min_log_mel)).exp() } else { f_sp * m }
    };
    let (lo, hi) = (hz_mel(0.0), hz_mel(8000.0));
    let noktalar: Vec<f64> = (0..N_MEL + 2).map(|i| mel_hz(lo + (hi - lo) * i as f64 / (N_MEL + 1) as f64)).collect();
    let fft_f: Vec<f64> = (0..nf).map(|i| i as f64 * SR as f64 / n_fft as f64).collect();
    let mut w = vec![0f32; N_MEL * nf];
    for m in 0..N_MEL {
        let (a, b, c) = (noktalar[m], noktalar[m + 1], noktalar[m + 2]);
        let norm = 2.0 / (c - a);
        for (k, &f) in fft_f.iter().enumerate() {
            let v = ((f - a) / (b - a)).min((c - f) / (c - b)).max(0.0);
            w[m * nf + k] = (v * norm) as f32;
        }
    }
    w
}

impl Whisper {
    pub fn ac(klasor: &Path) -> Result<Self> {
        let (kodlayici, g1) = crate::onnx::oturum(&klasor.join("onnx/encoder_model_fp16.onnx"), true)?;
        let (cozucu, g2) = crate::onnx::oturum(&klasor.join("onnx/decoder_model_merged_fp16.onnx"), true)?;
        let tok = Tokenizer::from_file(klasor.join("tokenizer.json")).map_err(|e| anyhow::anyhow!("tokenizer: {e}"))?;
        let ayar: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(klasor.join("generation_config.json"))?)?;
        let sayilar = |k: &str| -> Vec<usize> {
            ayar[k].as_array().map(|a| a.iter().filter_map(|v| v.as_u64().map(|v| v as usize)).collect()).unwrap_or_default()
        };
        let hizalama = ayar["alignment_heads"]
            .as_array()
            .context("alignment_heads")?
            .iter()
            .map(|p| (p[0].as_u64().unwrap_or(0) as usize, p[1].as_u64().unwrap_or(0) as usize))
            .collect();
        Ok(Whisper {
            kodlayici,
            cozucu,
            tok,
            mel: mel_filtreleri(),
            stft: Stft::new(400, 160, false),
            bastirilan: sayilar("suppress_tokens"),
            hizalama,
            ekran_karti: g1 && g2,
        })
    }

    /// ≤ 30 sn ses → log-mel (128 × 3000), Whisper'ın hesabı
    fn log_mel(&self, x: &[f32]) -> Vec<f32> {
        let mut d = x[..x.len().min(ORNEK_30)].to_vec();
        d.resize(ORNEK_30, 0.0);
        let (t, sp) = self.stft.ileri(&d); // t = 3001; son kare atılır
        let nf = self.stft.frekans();
        let mut guc = vec![0f32; nf * KARE];
        for f in 0..nf {
            for k in 0..KARE {
                let o = (f * t + k) * 2;
                guc[f * KARE + k] = sp[o] * sp[o] + sp[o + 1] * sp[o + 1];
            }
        }
        let mut m = vec![0f32; N_MEL * KARE];
        for b in 0..N_MEL {
            for f in 0..nf {
                let w = self.mel[b * nf + f];
                if w != 0.0 {
                    for k in 0..KARE {
                        m[b * KARE + k] += w * guc[f * KARE + k];
                    }
                }
            }
        }
        for v in m.iter_mut() {
            *v = v.max(1e-10).log10();
        }
        let tepe = m.iter().cloned().fold(f32::MIN, f32::max);
        for v in m.iter_mut() {
            *v = ((*v).max(tepe - 8.0) + 4.0) / 4.0;
        }
        m
    }

    fn kodla(&mut self, mel: Vec<f32>) -> Result<DynValue> {
        let mut r = self.kodlayici.run(ort::inputs!["input_features" => Tensor::from_array(([1usize, N_MEL, KARE], mel))?])?;
        Ok(r.remove("last_hidden_state").context("kodlayıcı çıktısı")?)
    }

    /// Kod çözücüyü önbelleksiz çalıştırır (ilk adım ve hizalama için): tüm belirteçler tek seferde.
    fn coz_tam(&mut self, kod: &DynValue, ids: &[i64]) -> Result<ort::session::SessionOutputs<'_>> {
        let bos_d = || Tensor::from_array(([1usize, BASLIK, 0, BOYUT], Vec::<f32>::new()));
        let mut girdi: Vec<(String, SessionInputValue<'_>)> = vec![
            ("input_ids".into(), Tensor::from_array(([1usize, ids.len()], ids.to_vec()))?.into()),
            ("encoder_hidden_states".into(), kod.into()),
        ];
        for l in 0..KATMAN {
            for kind in ["decoder", "encoder"] {
                for kv in ["key", "value"] {
                    girdi.push((format!("past_key_values.{l}.{kind}.{kv}"), bos_d()?.into()));
                }
            }
        }
        girdi.push(("use_cache_branch".into(), Tensor::from_array(([1usize], vec![false]))?.into()));
        Ok(self.cozucu.run(girdi)?)
    }

    /// Açgözlü kod çözme (zaman damgasız). Metin belirteçleri döner.
    fn metin(&mut self, kod: &DynValue) -> Result<Vec<i64>> {
        let on = [SOT, EN, TRANSCRIBE, NO_TS];
        let mut cikti: Vec<i64> = Vec::new();
        // ilk adım: önbelleksiz; sonra önbellekli (present.* → past_key_values.*)
        let (mut onbellek, ilk) = {
            let r = self.coz_tam(kod, &on)?;
            let (sh, lg) = r["logits"].try_extract_tensor::<f32>()?;
            let v = sh[2] as usize;
            let son = lg[(on.len() - 1) * v..on.len() * v].to_vec();
            let mut ob: Vec<(String, DynValue)> = Vec::new();
            for l in 0..KATMAN {
                for kind in ["decoder", "encoder"] {
                    for kv in ["key", "value"] {
                        let ad = format!("present.{l}.{kind}.{kv}");
                        let (s, d) = r[ad.as_str()].try_extract_tensor::<f32>()?;
                        let shape: Vec<usize> = s.iter().map(|&x| x as usize).collect();
                        ob.push((format!("past_key_values.{l}.{kind}.{kv}"), Tensor::from_array((shape, d.to_vec()))?.into_dyn()));
                    }
                }
            }
            (ob, son)
        };
        let mut logit = ilk;
        for adim in 0..EN_COK_TOKEN {
            for &b in &self.bastirilan {
                if b < logit.len() {
                    logit[b] = f32::NEG_INFINITY;
                }
            }
            if adim == 0 {
                logit[220] = f32::NEG_INFINITY; // baştaki boşluk ve hemen bitiş yasak
                logit[EOT as usize] = f32::NEG_INFINITY;
            }
            for v in logit.iter_mut().skip(EOT as usize + 1) {
                *v = f32::NEG_INFINITY; // özel belirteçler ve zaman damgaları (bitiş hariç)
            }
            let sec = logit.iter().enumerate().fold((0, f32::MIN), |m, (i, &v)| if v > m.1 { (i, v) } else { m }).0 as i64;
            if sec == EOT || tekrar_dongusu(&cikti, sec) {
                break;
            }
            cikti.push(sec);
            let mut girdi: Vec<(String, SessionInputValue<'_>)> = vec![
                ("input_ids".into(), Tensor::from_array(([1usize, 1], vec![sec]))?.into()),
                ("encoder_hidden_states".into(), kod.into()),
            ];
            for (ad, v) in &onbellek {
                girdi.push((ad.clone(), v.into()));
            }
            girdi.push(("use_cache_branch".into(), Tensor::from_array(([1usize], vec![true]))?.into()));
            let r = self.cozucu.run(girdi)?;
            let (sh, lg) = r["logits"].try_extract_tensor::<f32>()?;
            logit = lg[..sh[2] as usize].to_vec();
            // yalnız kod çözücünün kendi önbelleği büyür; kodlayıcınınki (çapraz dikkat) sabit kalır
            for (ad, v) in onbellek.iter_mut() {
                if ad.contains(".decoder.") {
                    let p = ad.replace("past_key_values", "present");
                    let (s, d) = r[p.as_str()].try_extract_tensor::<f32>()?;
                    let shape: Vec<usize> = s.iter().map(|&x| x as usize).collect();
                    *v = Tensor::from_array((shape, d.to_vec()))?.into_dyn();
                }
            }
        }
        Ok(cikti)
    }

    /// Kelime zamanları (grup içi sn): OpenAI Whisper `find_alignment` + `merge_punctuations`.
    fn kelimeler(&mut self, kod: &DynValue, metin: &[i64], kare_say: usize) -> Result<Vec<Kelime>> {
        if metin.is_empty() {
            return Ok(Vec::new());
        }
        let mut ids = vec![SOT, EN, TRANSCRIBE, NO_TS];
        ids.extend_from_slice(metin);
        ids.push(EOT);
        let n_tok = ids.len();
        let nf = (kare_say / 2).clamp(1, 1500);
        let hizalama = self.hizalama.clone();
        // başlık başına [belirteç][zaman] ağırlıkları
        let mut basliklar: Vec<Vec<f32>> = Vec::new();
        {
            let r = self.coz_tam(kod, &ids)?;
            for &(l, h) in &hizalama {
                let (s, d) = r[format!("cross_attentions.{l}").as_str()].try_extract_tensor::<f32>()?;
                let (sq, fr) = (s[2] as usize, s[3] as usize);
                let mut w = vec![0f32; sq * nf];
                for t in 0..sq {
                    for f in 0..nf {
                        w[t * nf + f] = d[(h * sq + t) * fr + f];
                    }
                }
                basliklar.push(w);
            }
        }
        // belirteç ekseninde standartlaştır, zaman ekseninde 7'lik ortanca süzgeç, başlıkların ortalaması
        let mut matris = vec![0f32; n_tok * nf];
        for w in &basliklar {
            let mut z = w.clone();
            for f in 0..nf {
                let ort: f32 = (0..n_tok).map(|t| w[t * nf + f]).sum::<f32>() / n_tok as f32;
                let sd = ((0..n_tok).map(|t| (w[t * nf + f] - ort).powi(2)).sum::<f32>() / n_tok as f32).sqrt().max(1e-9);
                for t in 0..n_tok {
                    z[t * nf + f] = (w[t * nf + f] - ort) / sd;
                }
            }
            for t in 0..n_tok {
                for f in 0..nf {
                    let mut p: Vec<f32> = (0..7)
                        .map(|k| {
                            let j = f as i64 + k - 3;
                            let j = if j < 0 { -j } else if j >= nf as i64 { 2 * (nf as i64 - 1) - j } else { j };
                            z[t * nf + j.clamp(0, nf as i64 - 1) as usize]
                        })
                        .collect();
                    p.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                    matris[t * nf + f] += p[3] / basliklar.len() as f32;
                }
            }
        }
        // satırlar: ön ekin son belirteci (zaman damgasız) + metin belirteçleri (bitiş hariç)
        let bas = 3;
        let satir = n_tok - 1 - bas;
        let (ti, zi) = dtw(&matris[bas * nf..(bas + satir) * nf].iter().map(|v| -v).collect::<Vec<_>>(), satir, nf);
        let mut sicrama = Vec::new();
        for k in 0..ti.len() {
            if k == 0 || ti[k] != ti[k - 1] {
                sicrama.push(zi[k] as f64 / 50.0);
            }
        }
        // kelimelere böl: boşlukla başlayan ya da noktalama olan parça yeni kelime
        let mut kelime: Vec<(String, usize)> = Vec::new(); // (metin, belirteç sayısı)
        for &t in metin {
            let s = self.tok.decode(&[t as u32], false).unwrap_or_default();
            let nokta = !s.trim().is_empty() && s.trim().chars().all(|c| c.is_ascii_punctuation());
            if s.starts_with(' ') || nokta || kelime.is_empty() {
                kelime.push((s, 1));
            } else if let Some(k) = kelime.last_mut() {
                k.0.push_str(&s);
                k.1 += 1;
            }
        }
        let mut out = Vec::new();
        let mut i = 0usize;
        for (s, n) in kelime {
            let a = sicrama.get(i).copied().unwrap_or(0.0);
            let b = sicrama.get(i + n).copied().unwrap_or(a);
            out.push((a, b, s));
            i += n;
        }
        Ok(noktalama_birlestir(out))
    }

    /// Konuşmanın dili (Whisper dil belirleme, ilk 30 sn): ("en", "ja" …) ve olasılığı
    pub fn dil(&mut self, x16: &[f32]) -> Result<(String, f32)> {
        let mel = self.log_mel(x16);
        let kod = self.kodla(mel)?;
        let son: Vec<f32> = {
            let r = self.coz_tam(&kod, &[SOT])?;
            let (sh, lg) = r["logits"].try_extract_tensor::<f32>()?;
            lg[..sh[2] as usize].to_vec()
        };
        // dil belirteçleri: 50259 (<|en|>) … 50358 (<|yue|>)
        let (a, b) = (EN as usize, 50359usize.min(son.len()));
        let tepe = son[a..b].iter().cloned().fold(f32::MIN, f32::max);
        let us: Vec<f32> = son[a..b].iter().map(|x| (x - tepe).exp()).collect();
        let top: f32 = us.iter().sum();
        let (i, e) = us.iter().enumerate().fold((0usize, 0f32), |m, (i, &e)| if e > m.1 { (i, e) } else { m });
        let ad = self.tok.id_to_token((a + i) as u32).unwrap_or_default();
        Ok((ad.trim_matches(|c| c == '<' || c == '>' || c == '|').to_string(), e / top.max(1e-9)))
    }

    /// 16 kHz konuşma sesi → kelimeler (sn). ilerleme(0..1)
    pub fn yaziya_dok(&mut self, x16: &[f32], vad: &mut crate::vad::Vad, mut ilerleme: impl FnMut(f32)) -> Result<Vec<Kelime>> {
        let bolum = vad.bolumler(x16, &crate::vad::Ayarlar::default())?;
        // bölümleri ≤ 30 sn'lik gruplara topla; > 30 sn bölüm 30 sn'lik parçalara bölünür
        let mut parcalar: Vec<(usize, usize)> = Vec::new();
        for (a, b) in bolum {
            let mut s = a;
            while b - s > ORNEK_30 {
                parcalar.push((s, s + ORNEK_30));
                s += ORNEK_30;
            }
            parcalar.push((s, b));
        }
        let mut gruplar: Vec<Vec<(usize, usize)>> = Vec::new();
        let mut boy = 0;
        for p in parcalar {
            let n = p.1 - p.0;
            if gruplar.is_empty() || boy + n > ORNEK_30 {
                gruplar.push(Vec::new());
                boy = 0;
            }
            gruplar.last_mut().unwrap().push(p);
            boy += n;
        }
        let mut sonuc = Vec::new();
        let toplam = gruplar.len().max(1);
        for (gi, g) in gruplar.into_iter().enumerate() {
            crate::ilerleme::kontrol()?;
            let mut ses = Vec::new();
            let mut harita = Vec::new(); // (grup içi başlangıç, gerçek başlangıç, uzunluk)
            for (a, b) in &g {
                harita.push((ses.len(), *a, b - a));
                ses.extend_from_slice(&x16[*a..*b]);
            }
            let z = std::time::Instant::now();
            let mel = self.log_mel(&ses);
            let z1 = z.elapsed().as_secs_f32();
            let kod = self.kodla(mel)?;
            let z2 = z.elapsed().as_secs_f32();
            let metin = self.metin(&kod)?;
            let z3 = z.elapsed().as_secs_f32();
            let kare_say = ses.len() / 160;
            let kel = self.kelimeler(&kod, &metin, kare_say)?;
            if std::env::var_os("SLATEVOX_ZAMAN").is_some() {
                eprintln!("[whisper] mel {z1:.2} · kodlayıcı {:.2} · metin {:.2} ({} belirteç) · hizalama {:.2} sn",
                          z2 - z1, z3 - z2, metin.len(), z.elapsed().as_secs_f32() - z3);
            }
            for (a, b, s) in kel {
                let cevir = |t: f64| {
                    let o = (t * SR as f64) as usize;
                    let (gb, gr, n) = harita.iter().rev().find(|h| h.0 <= o).copied().unwrap_or(harita[0]);
                    (gr + (o - gb).min(n)) as f64 / SR as f64
                };
                sonuc.push((cevir(a), cevir(b).max(cevir(a)), s));
            }
            ilerleme((gi + 1) as f32 / toplam as f32);
        }
        Ok(sonuc)
    }
}

/// Aynı kısa belirteç dizisi art arda 4+ kez tekrar ediyorsa (Whisper'ın takılma döngüsü) dur.
fn tekrar_dongusu(c: &[i64], yeni: i64) -> bool {
    let mut d = c.to_vec();
    d.push(yeni);
    for p in 1..=8 {
        if d.len() >= p * 4 && (1..4).all(|r| d[d.len() - p..] == d[d.len() - p * (r + 1)..d.len() - p * r]) {
            return true;
        }
    }
    false
}

/// OpenAI Whisper'ın dtw_cpu'su: (metin sırası, zaman sırası) yolu
fn dtw(x: &[f32], n: usize, m: usize) -> (Vec<usize>, Vec<usize>) {
    let inf = f32::INFINITY;
    let mut maliyet = vec![inf; (n + 1) * (m + 1)];
    let mut iz = vec![0u8; (n + 1) * (m + 1)];
    maliyet[0] = 0.0;
    for j in 1..=m {
        for i in 1..=n {
            let c0 = maliyet[(i - 1) * (m + 1) + j - 1];
            let c1 = maliyet[(i - 1) * (m + 1) + j];
            let c2 = maliyet[i * (m + 1) + j - 1];
            let (c, t) = if c0 < c1 && c0 < c2 { (c0, 0) } else if c1 < c0 && c1 < c2 { (c1, 1) } else { (c2, 2) };
            maliyet[i * (m + 1) + j] = x[(i - 1) * m + j - 1] + c;
            iz[i * (m + 1) + j] = t;
        }
    }
    for j in 0..=m {
        iz[j] = 2;
    }
    for i in 0..=n {
        iz[i * (m + 1)] = 1;
    }
    let (mut i, mut j) = (n, m);
    let (mut ti, mut zi) = (Vec::new(), Vec::new());
    while i > 0 || j > 0 {
        ti.push(i.saturating_sub(1));
        zi.push(j.saturating_sub(1));
        match iz[i * (m + 1) + j] {
            0 => {
                i -= 1;
                j -= 1;
            }
            1 => i -= 1,
            _ => j -= 1,
        }
    }
    ti.reverse();
    zi.reverse();
    (ti, zi)
}

/// Whisper'ın merge_punctuations: önündeki tırnak / parantez sonraki kelimeye, ardındaki nokta / virgül öncekine
fn noktalama_birlestir(mut k: Vec<Kelime>) -> Vec<Kelime> {
    const ONCE: &str = "\"'“¿([{-";
    const SONRA: &str = "\"'.。,，!！?？:：”)]}、";
    let mut i = k.len() as i64 - 2;
    while i >= 0 {
        let (a, b) = (i as usize, i as usize + 1);
        let t = k[a].2.trim().to_string();
        if k[a].2.starts_with(' ') && !t.is_empty() && t.chars().all(|c| ONCE.contains(c)) {
            let s = k[a].2.clone();
            k[b].2 = s + k[b].2.trim_start();
            k[b].0 = k[a].0;
            k.remove(a);
        }
        i -= 1;
    }
    let mut i = 1;
    while i < k.len() {
        let t = k[i].2.clone();
        if !t.starts_with(' ') && !t.is_empty() && t.chars().all(|c| SONRA.contains(c)) {
            k[i - 1].2.push_str(&t);
            k[i - 1].1 = k[i].1;
            k.remove(i);
        } else {
            i += 1;
        }
    }
    k
}
