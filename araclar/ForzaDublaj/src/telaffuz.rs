//! İngilizce adların Türkçe okunuşu (ses modeline giden metin; altyazı değişmez) — eski `telaffuz.py`'nin karşılığı.
//!   "Michael'ın arabası" → "Maykıl'ın arabası"
//! Değişen kelime üç şartı birden sağlar: İngilizce asılda da aynen geçiyor; yazılışı İngilizceye özgü (w, q, x, j,
//! th, sh, ee, sonda sessiz e…); kısaltma değil. Okunuş: trnorm'un marka sözlüğü → hazır terimler → CMU Pronouncing
//! Dictionary (BSD) → g2p_en (Kyubyong Park, Apache-2.0; küçük GRU, `modeller/g2p/g2p.bin`). Dönüşüm tablosu ve
//! kurallar: sewox/turkish-neural-tts (Apache-2.0).

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::LazyLock;

use anyhow::{Context, Result};
use fancy_regex::Regex;

static KELIME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?<![\w@./])([A-Za-zÇĞİÖŞÜçğıöşü]+)((?:['’][a-zçğıöşü]+)?)(?![\w@]|[./:]\w)").unwrap());
static INGILIZCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)[wqxj]|c(?!h)|th|sh|ch|ph|gh|ck|wh|kn|wr|ee|oo|ea|ou|oa|ai|ay|ey|ie|ue|au|aw|ew|ow|oy|[^aeiou]y(?:[^aeiou]|$)|[^aeiou]e$|er$|tion$|ight").unwrap()
});
static EN_KELIME: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"[A-Za-z]+").unwrap());
static UCLU: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(.)\1{2,}").unwrap());

const HAZIR: [(&str, &str); 28] = [
    ("jira", "jira"), ("java", "cava"), ("javascript", "cava skript"), ("typescript", "tayp skript"), ("linux", "linuks"), ("python", "paytın"),
    ("azure", "ejır"), ("github", "git hab"), ("gitlab", "git lab"), ("docker", "dakır"), ("kubernetes", "kubernetis"), ("teams", "tims"),
    ("slack", "slek"), ("zoom", "zum"), ("excel", "eksel"), ("outlook", "autluk"), ("google", "gugıl"), ("microsoft", "maykrosoft"),
    ("notion", "noşın"), ("confluence", "konfluens"), ("sonicwall", "sonik vol"), ("salesforce", "seylsfors"), ("whatsapp", "vatsap"),
    ("linkedin", "linkdin"), ("youtube", "yutub"), ("iphone", "ayfon"), ("macbook", "mekbuk"), ("windows", "vindovs"),
];

fn ses(p: &str) -> &'static str {
    match p {
        "AA" => "a",
        "AE" => "e",
        "AH" => "a",
        "AO" => "o",
        "AW" => "av",
        "AY" => "ay",
        "B" => "b",
        "CH" => "ç",
        "D" | "DH" => "d",
        "EH" => "e",
        "ER" => "ır",
        "EY" => "ey",
        "F" => "f",
        "G" => "g",
        "HH" => "h",
        "IH" | "IY" => "i",
        "JH" => "c",
        "K" => "k",
        "L" => "l",
        "M" => "m",
        "N" => "n",
        "NG" => "ng",
        "OW" => "o",
        "OY" => "oy",
        "P" => "p",
        "R" => "r",
        "S" => "s",
        "SH" => "ş",
        "T" | "TH" => "t",
        "UH" | "UW" => "u",
        "V" | "W" => "v",
        "Y" => "y",
        "Z" => "z",
        "ZH" => "j",
        _ => "",
    }
}

/// ARPAbet → Türkçe yazım
pub fn arpabet_tr(sesler: &[String], yazilis: &str) -> String {
    let low = yazilis.to_lowercase();
    let o_a_degil = low.contains('o') && !low.contains('a');
    let taban = |p: &str| p.trim_end_matches(['0', '1', '2']).to_string();
    let mut out: Vec<&str> = Vec::new();
    for (i, p) in sesler.iter().enumerate() {
        let b = taban(p);
        if p == "AH0" {
            out.push("ı");
        } else if b == "AA" && o_a_degil {
            out.push("o");
        } else if b == "NG" {
            let sonraki = sesler.get(i + 1).map(|x| taban(x)).unwrap_or_default();
            out.push(if sonraki == "K" || sonraki == "G" { "n" } else { "ng" });
        } else {
            out.push(ses(&b));
        }
    }
    if out.last() == Some(&"z") && low.ends_with('s') {
        *out.last_mut().unwrap() = "s";
    }
    UCLU.replace_all(&out.concat(), "$1$1").into_owned()
}

// ------------------------------------------------------------------ g2p_en
const HARF: &str = "abcdefghijklmnopqrstuvwxyz";
const FONEM: [&str; 74] = [
    "<pad>", "<unk>", "<s>", "</s>", "AA0", "AA1", "AA2", "AE0", "AE1", "AE2", "AH0", "AH1", "AH2", "AO0", "AO1", "AO2", "AW0", "AW1", "AW2", "AY0", "AY1", "AY2", "B",
    "CH", "D", "DH", "EH0", "EH1", "EH2", "ER0", "ER1", "ER2", "EY0", "EY1", "EY2", "F", "G", "HH", "IH0", "IH1", "IH2", "IY0", "IY1", "IY2", "JH", "K", "L", "M", "N",
    "NG", "OW0", "OW1", "OW2", "OY0", "OY1", "OY2", "P", "R", "S", "SH", "T", "TH", "UH0", "UH1", "UH2", "UW", "UW0", "UW1", "UW2", "V", "W", "Y", "Z", "ZH",
];

pub struct G2p {
    v: HashMap<String, (Vec<usize>, Vec<f32>)>,
}

impl G2p {
    pub fn ac(klasor: &Path) -> Result<Self> {
        let idx: Vec<(String, Vec<usize>, usize)> = serde_json::from_str(&std::fs::read_to_string(klasor.join("g2p.json"))?)?;
        let ham = std::fs::read(klasor.join("g2p.bin"))?;
        let f: Vec<f32> = ham.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
        let v = idx
            .into_iter()
            .map(|(ad, sekil, bas)| {
                let n: usize = sekil.iter().product();
                (ad, (sekil, f[bas..bas + n].to_vec()))
            })
            .collect();
        Ok(G2p { v })
    }

    fn d(&self, ad: &str) -> &(Vec<usize>, Vec<f32>) {
        &self.v[ad]
    }

    /// GRU hücresi (PyTorch sırası: r, z, n)
    fn hucre(&self, x: &[f32], h: &[f32], on: &str) -> Vec<f32> {
        let (wi, wh, bi, bh) = (self.d(&format!("{on}_w_ih")), self.d(&format!("{on}_w_hh")), self.d(&format!("{on}_b_ih")), self.d(&format!("{on}_b_hh")));
        let (g, hd) = (wi.0[0], h.len());
        let carp = |w: &(Vec<usize>, Vec<f32>), v: &[f32], b: &[f32]| -> Vec<f32> {
            let k = w.0[1];
            (0..g).map(|r| b[r] + w.1[r * k..(r + 1) * k].iter().zip(v).map(|(a, c)| a * c).sum::<f32>()).collect()
        };
        let gi = carp(wi, x, &bi.1);
        let gh = carp(wh, h, &bh.1);
        let s = |v: f32| 1.0 / (1.0 + (-v).exp());
        (0..hd)
            .map(|j| {
                let r = s(gi[j] + gh[j]);
                let z = s(gi[hd + j] + gh[hd + j]);
                let n = (gi[2 * hd + j] + r * gh[2 * hd + j]).tanh();
                (1.0 - z) * n + z * h[j]
            })
            .collect()
    }

    fn gomme(&self, ad: &str, i: usize) -> Vec<f32> {
        let e = self.d(ad);
        let k = e.0[1];
        e.1[i * k..(i + 1) * k].to_vec()
    }

    /// Sözlükte olmayan kelimenin ARPAbet tahmini
    pub fn sesler(&self, kelime: &str) -> Vec<String> {
        let mut ids: Vec<usize> = kelime.to_lowercase().chars().filter(|c| c.is_ascii()).map(|c| HARF.find(c).map_or(1, |p| p + 3)).collect();
        ids.push(2);
        let hd = self.d("enc_w_hh").0[1];
        let mut h = vec![0f32; hd];
        for i in ids {
            h = self.hucre(&self.gomme("enc_emb", i), &h, "enc");
        }
        let (fw, fb) = (self.d("fc_w"), self.d("fc_b"));
        let mut dec = self.gomme("dec_emb", 2);
        let mut out = Vec::new();
        for _ in 0..24 {
            h = self.hucre(&dec, &h, "dec");
            let k = fw.0[1];
            let p = (0..fw.0[0])
                .map(|r| fb.1[r] + fw.1[r * k..(r + 1) * k].iter().zip(&h).map(|(a, b)| a * b).sum::<f32>())
                .enumerate()
                .fold((0, f32::MIN), |m, (i, v)| if v > m.1 { (i, v) } else { m })
                .0;
            if p == 3 {
                break;
            }
            out.push(FONEM[p].to_string());
            dec = self.gomme("dec_emb", p);
        }
        out.into_iter().filter(|p| !p.starts_with('<')).collect()
    }
}

// ------------------------------------------------------------------ dönüştürücü
pub struct Telaffuz {
    cmu: Option<HashMap<String, Vec<String>>>,
    g2p: Option<G2p>,
}

pub fn cmu_oku(dosya: &Path) -> Result<HashMap<String, Vec<String>>> {
    let metin = std::fs::read_to_string(dosya).with_context(|| format!("CMU sözlüğü: {}", dosya.display()))?;
    let mut d = HashMap::new();
    for satir in metin.lines() {
        let s = satir.split('#').next().unwrap_or("").trim();
        let mut p = s.split_whitespace();
        let Some(w) = p.next() else { continue };
        if w.contains('(') {
            continue;
        }
        d.entry(w.to_string()).or_insert_with(|| p.map(str::to_string).collect());
    }
    Ok(d)
}

fn deve_parcalari(w: &str) -> Vec<String> {
    let mut p: Vec<String> = Vec::new();
    for c in w.chars() {
        if p.is_empty() || (c.is_uppercase() && p.last().unwrap().chars().last().is_some_and(char::is_lowercase)) {
            p.push(String::new());
        }
        p.last_mut().unwrap().push(c);
    }
    p
}

/// Ekin ünlülerini yeni okunuşun son ünlüsüne uydurur: Brooklyn'de → Bruklın'da
fn uyum(kok: &str, ek: &str) -> String {
    let (arka, on) = ("aıou", "eiöü");
    let Some(mut son) = kok.to_lowercase().chars().rev().find(|c| arka.contains(*c) || on.contains(*c)) else { return ek.to_string() };
    ek.chars()
        .map(|c| {
            let c = if "ae".contains(c) {
                if arka.contains(son) { 'a' } else { 'e' }
            } else if "ıiuü".contains(c) {
                match son {
                    'a' | 'ı' => 'ı',
                    'o' | 'u' => 'u',
                    'e' | 'i' => 'i',
                    _ => 'ü',
                }
            } else {
                c
            };
            if arka.contains(c) || on.contains(c) {
                son = c;
            }
            c
        })
        .collect()
}

fn buyut(w: &str, gibi: &str) -> String {
    if gibi.chars().next().is_some_and(char::is_uppercase) {
        let mut c = w.chars();
        let f = c.next().map(|f| match f {
            'i' => "İ".to_string(),
            'ı' => "I".to_string(),
            f => f.to_uppercase().to_string(),
        });
        f.unwrap_or_default() + c.as_str()
    } else {
        w.to_string()
    }
}

impl Telaffuz {
    pub fn new(modeller: &Path, cmu: bool, g2p: bool) -> Result<Self> {
        Ok(Telaffuz {
            cmu: if cmu { Some(cmu_oku(&modeller.join("cmudict/cmudict.dict"))?) } else { None },
            g2p: if g2p { Some(G2p::ac(&modeller.join("g2p"))?) } else { None },
        })
    }

    fn sozlukten(&self, w: &str) -> Option<String> {
        let low = w.to_lowercase();
        if let Some(c) = &self.cmu {
            if let Some(s) = c.get(&low) {
                return Some(arpabet_tr(s, w));
            }
            let ch: Vec<char> = low.chars().collect();
            for i in (2..ch.len().saturating_sub(1)).rev() {
                let (a, b): (String, String) = (ch[..i].iter().collect(), ch[i..].iter().collect());
                if let (Some(x), Some(y)) = (c.get(&a), c.get(&b)) {
                    return Some(arpabet_tr(x, &a) + &arpabet_tr(y, &b));
                }
            }
        }
        if let Some(g) = &self.g2p {
            let s = g.sesler(&low);
            if !s.is_empty() {
                return Some(arpabet_tr(&s, w));
            }
        }
        None
    }

    /// Çeviri cümlesindeki İngilizce adları Türkçe okunuşla yazar → (yeni metin, [(eski, yeni)])
    pub fn donustur(&self, tr: &str, en: &str) -> (String, Vec<(String, String)>) {
        if (self.cmu.is_none() && self.g2p.is_none()) || tr.is_empty() {
            return (tr.to_string(), Vec::new());
        }
        let en_k: HashSet<String> = EN_KELIME.find_iter(en).map(|m| m.as_str().to_lowercase()).collect();
        let hazir: HashMap<&str, &str> = HAZIR.into_iter().collect();
        let (mut out, mut pos, mut deg) = (String::new(), 0, Vec::new());
        for c in KELIME.captures_iter(tr).flatten() {
            let (m, kok, ek) = (c.get(0).unwrap(), c.get(1).unwrap().as_str(), c.get(2).map_or("", |x| x.as_str()));
            if kok.chars().count() < 2 || !kok.is_ascii() || !en_k.contains(&kok.to_lowercase()) || (kok.chars().all(|x| x.is_uppercase()) && kok.len() >= 2) {
                continue;
            }
            let deve = deve_parcalari(kok).len() > 1;
            let yeni = if let Some(b) = crate::trnorm::marka(kok) {
                Some(b)
            } else if let Some(h) = hazir.get(kok.to_lowercase().as_str()) {
                Some(h.to_string())
            } else if deve || INGILIZCE.is_match(kok).unwrap_or(false) {
                let p: Vec<Option<String>> = deve_parcalari(kok).iter().map(|p| self.sozlukten(p)).collect();
                if p.iter().all(Option::is_some) { Some(p.into_iter().flatten().collect::<Vec<_>>().join(" ")) } else { None }
            } else {
                None
            };
            let Some(yeni) = yeni else { continue };
            if yeni.to_lowercase() == kok.to_lowercase() {
                continue;
            }
            let yeni = buyut(&yeni, kok);
            let yek = if ek.is_empty() {
                String::new()
            } else {
                let mut ch = ek.chars();
                let ilk = ch.next().unwrap();
                format!("{ilk}{}", uyum(&yeni, ch.as_str()))
            };
            out.push_str(&tr[pos..m.start()]);
            out.push_str(&format!("{yeni}{yek}"));
            pos = m.end();
            deg.push((format!("{kok}{ek}"), format!("{yeni}{yek}")));
        }
        out.push_str(&tr[pos..]);
        (out, deg)
    }
}
