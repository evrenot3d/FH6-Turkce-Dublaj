//! Ses temeli: WAV okuma / yazma, yeniden örnekleme, ses gücü, sessizlik kırpma ve ffmpeg çağrıları.
//! Ses her yerde kanal kanal ayrı dizilerde (`Vec<Vec<f32>>`, -1..1) tutulur.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use anyhow::{Context, Result, bail};

/// Kanal kanal ses + örnekleme hızı.
#[derive(Clone, Debug)]
pub struct Ses {
    pub sr: u32,
    pub kanallar: Vec<Vec<f32>>,
}

impl Ses {
    pub fn uzunluk(&self) -> usize {
        self.kanallar.first().map_or(0, Vec::len)
    }

    /// Kanalların ortalaması (tek kanal).
    pub fn mono(&self) -> Vec<f32> {
        let n = self.kanallar.len().max(1) as f32;
        (0..self.uzunluk()).map(|i| self.kanallar.iter().map(|k| k[i]).sum::<f32>() / n).collect()
    }
}

pub fn wav_oku(yol: &Path) -> Result<Ses> {
    let r = hound::WavReader::open(yol).with_context(|| format!("WAV açılamadı: {}", yol.display()))?;
    wav_coz(r)
}

/// Bellekteki WAV baytları (sunuculardan gelen ses)
pub fn wav_oku_bayt(b: &[u8]) -> Result<Ses> {
    let r = hound::WavReader::new(std::io::Cursor::new(b)).context("WAV çözülemedi")?;
    wav_coz(r)
}

fn wav_coz<R: std::io::Read>(mut r: hound::WavReader<R>) -> Result<Ses> {
    let s = r.spec();
    let ch = s.channels as usize;
    let duz: Vec<f32> = match (s.sample_format, s.bits_per_sample) {
        (hound::SampleFormat::Float, 32) => r.samples::<f32>().collect::<Result<_, _>>()?,
        (hound::SampleFormat::Int, 16) => r.samples::<i16>().map(|v| v.map(|v| v as f32 / 32768.0)).collect::<Result<_, _>>()?,
        (hound::SampleFormat::Int, b) => {
            let o = (1i64 << (b - 1)) as f32;
            r.samples::<i32>().map(|v| v.map(|v| v as f32 / o)).collect::<Result<_, _>>()?
        }
        (f, b) => bail!("desteklenmeyen WAV biçimi: {f:?} {b} bit"),
    };
    let n = duz.len() / ch.max(1);
    let kanallar = (0..ch).map(|c| (0..n).map(|i| duz[i * ch + c]).collect()).collect();
    Ok(Ses { sr: s.sample_rate, kanallar })
}

/// 16 bit PCM WAV yazar (önce geçici dosyaya, sonra yerine taşır: yarım dosya kalmaz).
pub fn wav_yaz(yol: &Path, kanallar: &[Vec<f32>], sr: u32) -> Result<()> {
    let ch = kanallar.len().max(1);
    let spec = hound::WavSpec { channels: ch as u16, sample_rate: sr, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let gecici = yol.with_extension("tmp.wav");
    {
        let mut w = hound::WavWriter::create(&gecici, spec)?;
        let n = kanallar.first().map_or(0, Vec::len);
        for i in 0..n {
            for k in kanallar {
                w.write_sample((k[i].clamp(-1.0, 1.0) * 32767.0) as i16)?;
            }
        }
        w.finalize()?;
    }
    std::fs::rename(&gecici, yol)?;
    Ok(())
}

pub fn wav_yaz_mono(yol: &Path, x: &[f32], sr: u32) -> Result<()> {
    wav_yaz(yol, std::slice::from_ref(&x.to_vec()), sr)
}

// ------------------------------------------------------------------ yeniden örnekleme
fn ebob(a: u32, b: u32) -> u32 {
    if b == 0 { a } else { ebob(b, a % b) }
}

fn bessel_i0(x: f64) -> f64 {
    let (mut s, mut t, mut k) = (1.0, 1.0, 1.0);
    while t > 1e-12 * s {
        t *= (x / (2.0 * k)).powi(2);
        s += t;
        k += 1.0;
    }
    s
}

/// Çok fazlı (polyphase) pencereli sinc yeniden örnekleme (Kaiser β=8,6; ~32 sıfır geçişi). Oran L/M tam sayı
/// kesrine indirgenir (48 k → 16 k: 1/3, 24 k → 48 k: 2/1). Kalite soxr / librosa'ya yakın; başta / sonda gecikme yok.
pub fn yeniden_ornekle(x: &[f32], sr: u32, hedef: u32) -> Vec<f32> {
    if sr == hedef || x.is_empty() {
        return x.to_vec();
    }
    let g = ebob(sr, hedef);
    let (l, m) = ((hedef / g) as usize, (sr / g) as usize);
    let kesim = 0.97 * (1.0 / l.max(m) as f64);       // L katı hızda, en dar Nyquist'in biraz altı
    let yari = 32 * l.max(m);                         // L katı hızda yarı filtre uzunluğu
    let beta = 8.6;
    let i0b = bessel_i0(beta);
    let h: Vec<f64> = (0..=2 * yari)
        .map(|n| {
            let t = n as f64 - yari as f64;
            let sinc = if t == 0.0 { 1.0 } else { (std::f64::consts::PI * kesim * t).sin() / (std::f64::consts::PI * kesim * t) };
            let r = t / yari as f64;
            let w = bessel_i0(beta * (1.0 - r * r).max(0.0).sqrt()) / i0b;
            sinc * kesim * l as f64 * w
        })
        .collect();
    let n_out = ((x.len() as u64 * l as u64 + m as u64 - 1) / m as u64) as usize;
    let mut y = vec![0f32; n_out];
    use rayon::prelude::*;
    y.par_iter_mut().enumerate().for_each(|(j, out)| {
        // çıkış örneği j, L katı hızdaki k = j·M konumunda; giriş örneği i, L·i konumunda
        let k = (j * m) as i64;
        let i_min = ((k - yari as i64) as f64 / l as f64).ceil().max(0.0) as usize;
        let i_max = (((k + yari as i64) as f64 / l as f64).floor() as usize).min(x.len() - 1);
        let mut s = 0.0f64;
        for i in i_min..=i_max {
            let hi = (i as i64 * l as i64 - k + yari as i64) as usize;
            s += x[i] as f64 * h[hi];
        }
        *out = s as f32;
    });
    y
}

// ------------------------------------------------------------------ ölçüler
/// Konuşulan kısımların ses gücü (50 ms'lik pencerelerden en güçlünün %6'sını aşanlar).
pub fn etkin_rms(x: &[f32], sr: u32) -> f32 {
    let hop = ((sr as f32 * 0.05) as usize).max(1);
    let r: Vec<f32> = x.chunks_exact(hop).map(|c| (c.iter().map(|v| v * v).sum::<f32>() / hop as f32 + 1e-12).sqrt()).collect();
    let tepe = r.iter().cloned().fold(0.0, f32::max);
    let etkin: Vec<f32> = r.into_iter().filter(|&v| v > (tepe * 0.06).max(1e-4)).collect();
    if etkin.is_empty() { 0.0 } else { (etkin.iter().map(|v| v * v).sum::<f32>() / etkin.len() as f32).sqrt() }
}

/// Baştaki ve sondaki sessizliği kırpar (en yüksek genliğin %2'si eşik, 30 ms pay).
pub fn sessizligi_kirp(a: &[f32], sr: u32) -> Vec<f32> {
    let tepe = a.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let esik = (tepe * 0.02).max(1e-4);
    let (Some(ilk), Some(son)) = (a.iter().position(|v| v.abs() > esik), a.iter().rposition(|v| v.abs() > esik)) else {
        return Vec::new();
    };
    let p = (0.03 * sr as f32) as usize;
    a[ilk.saturating_sub(p)..(son + p).min(a.len())].to_vec()
}

// ------------------------------------------------------------------ ffmpeg
static FFMPEG: OnceLock<PathBuf> = OnceLock::new();

/// bin/ffmpeg.exe (yoksa PATH'teki ffmpeg)
pub fn ffmpeg_ayarla(bin: &Path) {
    let p = bin.join("ffmpeg.exe");
    let _ = FFMPEG.set(if p.is_file() { p } else { PathBuf::from("ffmpeg") });
}

pub fn ffmpeg_komut() -> Command {
    let mut c = Command::new(FFMPEG.get().cloned().unwrap_or_else(|| PathBuf::from("ffmpeg")));
    pencere_acma(&mut c);
    c
}

/// Alt programlar siyah konsol penceresi açmasın.
pub fn pencere_acma(c: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
}

/// ffmpeg'i verilen argümanlarla çalıştırır; hata olursa son hata satırı döner.
pub fn ffmpeg(args: &[&str]) -> Result<()> {
    let o = ffmpeg_komut().args(["-hide_banner", "-loglevel", "error", "-y"]).args(args).output()?;
    if !o.status.success() {
        let e = String::from_utf8_lossy(&o.stderr);
        bail!("ffmpeg: {}", e.lines().last().unwrap_or("bilinmeyen hata"));
    }
    Ok(())
}

/// `ffmpeg -i` çıktısı (biçim, akışlar, süre)
pub fn ffmpeg_bilgi(yol: &Path) -> Result<String> {
    ffmpeg_bilgi_girdi(&yol.to_string_lossy(), None)
}

fn ffmpeg_bilgi_girdi(girdi: &str, basliklar: Option<&str>) -> Result<String> {
    let mut c = ffmpeg_komut();
    c.arg("-hide_banner").args(girdi_secenekleri(girdi, basliklar, "20000000"));
    let o = c.arg("-i").arg(girdi).output()?;
    Ok(String::from_utf8_lossy(&o.stderr).into_owned())
}

/// `-i`'den önce gelen girdi seçenekleri: HTTP başlıkları, zaman aşımı ve gerekirse HLS ayarları.
/// Bazı siteler akış listesini (m3u8) uzantısız adresten "text/plain" diye, parçalarını da uzantısız ya da resim
/// uzantılı adreslerden verir; ffmpeg bunları kendiliğinden HLS saymaz ya da güvenlik için reddeder.
fn girdi_secenekleri(girdi: &str, basliklar: Option<&str>, zaman_asimi: &str) -> Vec<String> {
    let mut a: Vec<String> = Vec::new();
    if let Some(h) = basliklar {
        a.extend(["-headers".into(), h.into()]);
    }
    if girdi.starts_with("http") {
        a.extend(["-rw_timeout".into(), zaman_asimi.into()]);
        if hls_mi(girdi, basliklar) {
            a.extend(["-f".into(), "hls".into()]);
            a.extend(hls_secenekleri().iter().map(|s| s.to_string()));
        }
    }
    a
}

/// ffmpeg sürümüne göre: parça adreslerinin uzantısına bakılmasın
fn hls_secenekleri() -> &'static [&'static str] {
    static S: OnceLock<Vec<&'static str>> = OnceLock::new();
    S.get_or_init(|| {
        let yardim = ffmpeg_komut()
            .args(["-hide_banner", "-h", "demuxer=hls"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr))
            .unwrap_or_default();
        let mut v = Vec::new();
        if yardim.contains("extension_picky") {
            v.extend(["-extension_picky", "0"]);
        }
        if yardim.contains("allowed_extensions") {
            v.extend(["-allowed_extensions", "ALL"]);
        }
        v
    })
}

/// Adres bir HLS listesi mi (uzantısı .m3u8 değilse içeriğin başına bakılır; sonuç akılda tutulur)
fn hls_mi(url: &str, basliklar: Option<&str>) -> bool {
    use std::collections::HashMap;
    use std::io::Read;
    use std::sync::Mutex;
    static BILINEN: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
    let yol = url.split(['?', '#']).next().unwrap_or("").to_ascii_lowercase();
    if yol.ends_with(".m3u8") || yol.ends_with(".m3u") {
        return true;
    }
    if [".mp4", ".webm", ".mkv", ".mov", ".m4a", ".mp3", ".ogg", ".opus", ".flv", ".mpd", ".ts", ".aac", ".wav"]
        .iter()
        .any(|u| yol.ends_with(u))
    {
        return false;
    }
    let bilinen = BILINEN.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(&b) = bilinen.lock().unwrap().get(url) {
        return b;
    }
    let mut istek = crate::surec::ajan(std::time::Duration::from_secs(15)).get(url);
    for satir in basliklar.unwrap_or("").split("\r\n") {
        if let Some((ad, deger)) = satir.split_once(':') {
            let ad = ad.trim();
            if !ad.is_empty() && !ad.eq_ignore_ascii_case("host") && !ad.eq_ignore_ascii_case("range") {
                istek = istek.header(ad, deger.trim());
            }
        }
    }
    let sonuc = istek
        .call()
        .ok()
        .filter(|r| r.status().as_u16() < 400)
        .map(|mut r| {
            let mut b = [0u8; 64];
            let mut n = 0;
            let mut okuyucu = r.body_mut().as_reader();
            while n < b.len() {
                match okuyucu.read(&mut b[n..]) {
                    Ok(0) | Err(_) => break,
                    Ok(k) => n += k,
                }
            }
            String::from_utf8_lossy(&b[..n]).trim_start_matches('\u{feff}').trim_start().starts_with("#EXTM3U")
        })
        .unwrap_or(false);
    bilinen.lock().unwrap().insert(url.to_string(), sonuc);
    sonuc
}

/// Videonun / sesin süresi (sn).
pub fn sure_oku(yol: &Path) -> Result<f64> {
    sure_oku_girdi(&yol.to_string_lossy(), None)
}

/// Dosyanın ya da (HTTP başlıklarıyla) akış adresinin süresi (sn)
pub fn sure_oku_girdi(girdi: &str, basliklar: Option<&str>) -> Result<f64> {
    let b = ffmpeg_bilgi_girdi(girdi, basliklar)?;
    let Some(d) = b.split("Duration:").nth(1) else {
        // ffmpeg'in son hata satırı (ör. 403 Forbidden) sorunu anlatır
        let son = b.lines().rev().map(str::trim).find(|l| !l.is_empty() && !l.starts_with("At least one output")).unwrap_or("");
        bail!("Videonun süresi okunamadı. {son}");
    };
    let t: Vec<f64> = d.trim().split(',').next().unwrap_or("").split(':').filter_map(|v| v.trim().parse().ok()).collect();
    if t.len() != 3 {
        bail!("Videonun süresi okunamadı.");
    }
    Ok(t[0] * 3600.0 + t[1] * 60.0 + t[2])
}

/// Videonun [bas, bas+sure] aralığının sesi → WAV (sr Hz, kanal sayısı ch). Girdi dosya ya da akış adresi
/// (akışta ffmpeg yalnız bu aralığı çeker; HTTP başlıkları "Ad: değer, …").
pub fn ses_cikar(girdi: &str, basliklar: Option<&str>, bas: f64, sure: f64, sr: u32, ch: u16, cikis: &Path) -> Result<()> {
    let (b, s, r, c) = (format!("{bas:.3}"), format!("{sure:.3}"), sr.to_string(), ch.to_string());
    let o = cikis.to_string_lossy();
    let secenek = girdi_secenekleri(girdi, basliklar, "30000000");
    let mut a: Vec<&str> = secenek.iter().map(String::as_str).collect();
    a.extend(["-ss", &b, "-t", &s, "-i", girdi, "-vn", "-ac", &c, "-ar", &r, &o]);
    ffmpeg(&a)
}

/// Perdeyi bozmadan hızlandırır / yavaşlatır (ffmpeg atempo; f > 1 hızlı).
pub fn atempo(a: &[f32], sr: u32, f: f32, gecici: &Path) -> Result<Vec<f32>> {
    std::fs::create_dir_all(gecici)?;
    let (gi, ci) = (gecici.join("atempo_g.wav"), gecici.join("atempo_c.wav"));
    wav_yaz_mono(&gi, a, sr)?;
    let mut filtre = Vec::new();
    let mut k = f;
    while k > 2.0 {
        filtre.push("atempo=2.0".to_string());
        k /= 2.0;
    }
    while k < 0.5 {
        filtre.push("atempo=0.5".to_string());
        k /= 0.5;
    }
    filtre.push(format!("atempo={k:.4}"));
    ffmpeg(&["-i", &gi.to_string_lossy(), "-af", &filtre.join(","), &ci.to_string_lossy()])?;
    Ok(wav_oku(&ci)?.mono())
}

#[cfg(test)]
mod testler {
    use super::*;

    #[test]
    fn ornekleme_sinus_korunur() {
        // 48 kHz'de 1 kHz sinüs → 16 kHz → 48 kHz: genlik ve faz korunmalı
        let sr = 48000;
        let x: Vec<f32> = (0..sr).map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / sr as f32).sin() * 0.5).collect();
        let y = yeniden_ornekle(&x, sr, 16000);
        assert_eq!(y.len(), 16000);
        let z = yeniden_ornekle(&y, 16000, sr);
        let hata: f32 = x[2000..46000].iter().zip(&z[2000..46000]).map(|(a, b)| (a - b).powi(2)).sum::<f32>() / 44000.0;
        assert!(hata.sqrt() < 0.005, "hata {}", hata.sqrt());
    }
}
