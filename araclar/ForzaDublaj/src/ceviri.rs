//! İngilizce → Türkçe çeviri: Hy-MT2 7B (Tencent, GGUF Q4_K_M), llama.cpp sunucusuyla (`bin/llama-cuda/llama-server.exe`,
//! CUDA 13.5 derlemesi b12374; sığmayan katmanlar işlemcide). SlateVox'taki istem ve hece hedefli seçim aynen;
//! hız için 8 cümle aynı anda çevrilir (sunucuda 8 yuva, sürekli toplu işleme; ölçüm 15 Kasım 2026, RTX 5060:
//! 4 yuva 148, 8 yuva 205, 12 yuva 227 jeton/sn; 8 yuva kartta ~140 MB fazla yer tutar).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::{Result, bail};
use regex::Regex;
use serde_json::{Value, json};

use crate::cumle::hece;
use crate::surec::{Surec, ajan, yanit_al};
use crate::yollar::Yollar;

pub const PORT: u16 = 17893;
/// Model dosyaları, tercih sırasıyla (ilk bulunan kullanılır). Küçük olan (IQ4_XS, ~3,9 GB) 8 GB'lık kartta ses
/// modelleriyle birlikte tam sığar; Q4_K_M (4,4 GB) kart doluyken birkaç katmanı işlemciye taşar ve 2,5 kat yavaşlar
/// (ölçüm: 32/32 katman 205, 31/32 104, 30/32 80 jeton/sn).
/// Bulut sunucusunda (32 GB kart) daha hassas Q8_0 kullanılır; bu bilgisayarda yalnız Q4_K_M var.
const MODELLER: [&str; 3] = ["hy-mt2/Hy-MT2-7B-Q8_0.gguf", "hy-mt2/Hy-MT2-7B-IQ4_XS.gguf", "hy-mt2/Hy-MT2-7B-Q4_K_M.gguf"];
/// yuva başına bağlam (jeton): istem ~40 + en uzun cümle ~100 + çevirisi ~150
const YUVA_BAGLAMI: usize = 320;
const KATMAN: i64 = 32;
const YUVA: usize = 8;
const MT_ISTEM: &str = "Translate the following text into Turkish. Note that you should only output the translated result without any additional explanation:\n\n";

/// Hece hedefli çeviri: tutmayan cümle bu kadar farklı örneklemeyle yeniden çevrilir (SlateVox'taki gibi)
const HECE_DENEME: u64 = 3;

/// İngilizce süreye denk Türkçe hece: süre × ses modelinin konuşma hızı (hece / sn; ölçülmediyse 6)
pub fn hece_hedefi(sure: f64, hiz: f64) -> usize {
    ((sure * hiz).round() as usize).max(2)
}

fn hece_uygun(n: usize, hedef: usize) -> bool {
    let (n, t) = (n as f64, hedef as f64);
    0.7 * t <= n && n <= 1.15 * t + 1.0
}

static DUSUNME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<think>.*?</think>").unwrap());
static ONEK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(Türkçe|Turkish)\s*:\s*").unwrap());

pub fn temizle(out: &str, yedek: &str) -> String {
    let t = DUSUNME.replace_all(out, "");
    let t = t.trim().trim_matches(['"', '“', '”', '«', '»']).trim();
    let t = ONEK.replace(t, "").to_string();
    let t = t.lines().next().unwrap_or("").trim().to_string();
    if t.is_empty() { yedek.to_string() } else { t }
}

/// Ekran kartının boş belleği (MB), nvidia-smi ile
pub fn bos_vram_mb() -> Option<i64> {
    let mut c = std::process::Command::new("nvidia-smi");
    c.args(["--query-gpu=memory.total,memory.used", "--format=csv,noheader,nounits"]);
    crate::ses::pencere_acma(&mut c);
    let o = c.output().ok()?;
    let s = String::from_utf8_lossy(&o.stdout);
    let v: Vec<i64> = s.lines().next()?.split(',').filter_map(|x| x.trim().parse::<f64>().ok().map(|f| f as i64)).collect();
    (v.len() == 2).then(|| v[0] - v[1])
}

pub struct Ceviri {
    surec: Surec,
    model: PathBuf,
    /// v2 (bulut): Hy-MT2 30B vLLM sunucusu dışarıdan (linux/v2/sunucu.sh) açılır / kapatılır
    yollar: Yollar,
}

/// v2'de aynı anda çevrilen cümle sayısı (vLLM sürekli toplu işler; çok yuva hızlandırır)
fn yuva() -> usize {
    if crate::dis::etkin() { 32 } else { YUVA }
}

impl Ceviri {
    pub fn new(y: &Yollar) -> Self {
        Ceviri {
            // CUDA derlemesi (Vulkan'dakinden hızlı; Windows bellek sıkışınca Vulkan belleğini sistem belleğine atıp çeviriyi 6 kat yavaşlatıyordu)
            surec: Surec::new(
                "Hy-MT2",
                [y.bin.join("llama-cuda").join(crate::yollar::exe("llama-server")), y.bin.join("llama").join(crate::yollar::exe("llama-server"))]
                    .into_iter()
                    .find(|p| p.is_file())
                    .unwrap_or_default(),
                y.kok.join("llama-server.log"),
                format!("http://127.0.0.1:{PORT}/health"),
            ),
            model: model_yolu(&y.modeller),
            yollar: y.clone(),
        }
    }

    pub fn calisiyor(&mut self) -> bool {
        self.surec.calisiyor()
    }

    /// `ayir_mb`: ekran kartında başka modellere bırakılacak bellek
    pub fn baslat(&mut self, ayir_mb: i64) -> Result<()> {
        if crate::dis::etkin() {
            return crate::dis::komut(&self.yollar, &["baslat", "ceviri"]);
        }
        if self.surec.calisiyor() {
            return Ok(());
        }
        if !self.model.is_file() {
            bail!("Hy-MT2 bulunamadı: {}", self.model.display());
        }
        let boyut = (std::fs::metadata(&self.model)?.len() / (1 << 20)) as i64;
        let ek = 420; // bağlam + çalışma belleği (ölçüldü: 8 yuva, 4096 jeton bağlamla model + 496 MB; bağlam 2560'a indi)
        let ngl = match bos_vram_mb() {
            Some(bos) if bos - ayir_mb < boyut + ek => {
                let n = ((bos - ayir_mb - ek).max(0) as f64 / (boyut as f64 / KATMAN as f64)) as i64;
                eprintln!("[çeviri] Hy-MT2: {n}/{KATMAN} katman ekran kartında (boş {bos} MB, ayrılan {ayir_mb} MB)");
                n.clamp(0, 99)
            }
            _ => 99,
        };
        let args: Vec<OsString> = [
            "-m".into(),
            self.model.clone().into_os_string(),
            "--host".into(),
            "127.0.0.1".into(),
            "--port".into(),
            PORT.to_string().into(),
            "-ngl".into(),
            ngl.to_string().into(),
            "-c".into(),
            (YUVA_BAGLAMI * YUVA).to_string().into(),
            "-np".into(),
            YUVA.to_string().into(),
            "--jinja".into(),
            "-fa".into(),
            "on".into(),
            "-ctk".into(),
            "q8_0".into(),
            "-ctv".into(),
            "q8_0".into(),
            "-t".into(),
            "4".into(),
            "-tb".into(),
            "4".into(),
        ]
        .to_vec();
        self.surec.baslat(&args, &format!("ngl{ngl}"), Duration::from_secs(240))
    }

    pub fn durdur(&mut self) {
        if crate::dis::etkin() {
            let _ = crate::dis::komut(&self.yollar, &["durdur", "ceviri"]);
            return;
        }
        self.surec.durdur();
    }
}

fn sohbet(istem: &str, en_cok: usize, ek: Value) -> Result<String> {
    let mut govde = json!({"messages": [{"role": "user", "content": istem}], "temperature": 0.1, "top_p": 0.6, "top_k": 20,
                           "repeat_penalty": 1.05, "max_tokens": en_cok, "stream": false});
    if let Ok(m) = std::env::var("FD_CEVIRI_MODEL") {
        govde["model"] = json!(m);
    }
    if let (Some(g), Some(e)) = (govde.as_object_mut(), ek.as_object()) {
        for (k, v) in e {
            g.insert(k.clone(), v.clone());
        }
    }
    let r = ajan(Duration::from_secs(300)).post(&format!("http://127.0.0.1:{PORT}/v1/chat/completions")).send_json(&govde)?;
    let b = yanit_al("Hy-MT2", r)?;
    let v: Value = serde_json::from_slice(&b)?;
    Ok(v["choices"][0]["message"]["content"].as_str().unwrap_or("").to_string())
}

/// Tek cümle. `sure` verilirse (hece hedefli) sığmayan çeviri 3 farklı örneklemeyle yeniden denenir, hedefe en yakını
/// alınır. `hiz`: seçili ses modelinin konuşma hızı (hece / sn).
pub fn cevir_bir(en: &str, sure: Option<f64>, hiz: f64) -> Result<String> {
    let n = (en.len() * 3 + 64).min(YUVA_BAGLAMI - 120);
    let bir = |ornek: u64| -> Result<String> {
        let ek = if ornek > 0 { json!({"temperature": 0.8, "top_p": 0.95, "seed": ornek}) } else { json!({}) };
        Ok(temizle(&sohbet(&format!("{MT_ISTEM}{en}"), n, ek)?, en))
    };
    let tr = bir(0)?;
    let Some(d) = sure else { return Ok(tr) };
    let hedef = hece_hedefi(d, hiz);
    if tr.trim().is_empty() || hece_uygun(hece(&tr), hedef) {
        return Ok(tr);
    }
    let mut aday = vec![tr.clone()];
    for k in 1..=HECE_DENEME {
        if let Ok(a) = bir(k) {
            aday.push(a);
        }
    }
    let ilk = tr.trim().chars().count().max(1) as f64;
    let en_iyi = aday
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            let a = c.trim().chars().count() as f64;
            !c.trim().is_empty() && 0.45 * ilk <= a && a <= 2.2 * ilk
        })
        .min_by_key(|(k, c)| (!hece_uygun(hece(c), hedef), (hece(c) as i64 - hedef as i64).abs(), *k))
        .map(|(_, c)| c.clone())
        .unwrap_or(tr);
    Ok(en_iyi)
}

/// Bütün cümleler, 8'i aynı anda. Her biten cümle `bitti(sıra, çeviri)` ile hemen bildirilir (seslendirme beklemesin).
pub fn cevir_hepsi(en: &[String], sureler: Option<&[f64]>, hiz: f64, bitti: &(dyn Fn(usize, String) + Sync)) {
    let sira = AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..yuva().min(en.len()) {
            s.spawn(|| {
                loop {
                    let i = sira.fetch_add(1, Ordering::SeqCst);
                    if i >= en.len() {
                        break;
                    }
                    let t = en[i].trim();
                    let tr = if t.is_empty() {
                        String::new()
                    } else {
                        cevir_bir(t, sureler.and_then(|s| s.get(i).copied()), hiz).unwrap_or_else(|e| {
                            eprintln!("[çeviri] cümle {}: {e}", i + 1);
                            String::new()
                        })
                    };
                    bitti(i, tr);
                }
            });
        }
    });
}

fn model_yolu(modeller: &Path) -> PathBuf {
    MODELLER.iter().map(|m| modeller.join(m)).find(|p| p.is_file()).unwrap_or_else(|| modeller.join(MODELLER[2]))
}
