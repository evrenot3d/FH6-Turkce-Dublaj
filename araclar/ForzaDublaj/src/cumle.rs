//! Whisper kelimelerinden cümleler ve Türkçe hece sayısı.

use serde::{Deserialize, Serialize};

use crate::whisper::Kelime;

/// Bir cümle: videodaki (parça içi) başlangıç / bitiş (sn) ve İngilizce metin. JSON adları eski sürümle aynı.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Cumle {
    pub start: f64,
    pub end: f64,
    pub en: String,
}

impl Cumle {
}

fn kisaltma(t: &str) -> bool {
    let k = t.to_lowercase();
    ["mr.", "mrs.", "ms.", "dr.", "st.", "vs.", "etc.", "jr.", "sr.", "e.g.", "i.e.", "u.s."].contains(&k.as_str())
}

/// Kelimeler → cümleler. `kesmeler`: konuşanın değiştiği kelime sıraları (orada yeni cümle başlar).
/// Kurallar (eski sürümle aynı): 1 sn'den uzun sessizlik; nokta / soru / ünlem (≥ 0,8 sn); 9 sn'yi geçince virgülde;
/// 14 sn'yi geçince her yerde; noktalamasız (Whisper noktalamayı kaybetmiş) 3 sn'yi geçen cümle 0,6 sn duraklamada.
pub fn cumleler(kelimeler: &[Kelime], kesmeler: &std::collections::HashSet<usize>) -> Vec<Cumle> {
    let mut out = Vec::new();
    let mut cur: Vec<&Kelime> = Vec::new();
    let bosalt = |cur: &mut Vec<&Kelime>, out: &mut Vec<Cumle>| {
        if !cur.is_empty() {
            let metin = cur.iter().map(|k| k.2.as_str()).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ");
            if !metin.is_empty() {
                out.push(Cumle { start: (cur[0].0 * 1000.0).round() / 1000.0, end: (cur[cur.len() - 1].1 * 1000.0).round() / 1000.0, en: metin });
            }
            cur.clear();
        }
    };
    for (k, w) in kelimeler.iter().enumerate() {
        if let Some(son) = cur.last() {
            let ara = w.0 - son.1;
            let ciplak = !cur.iter().any(|x| x.2.contains(['.', '?', '!', ',', ';', ':']));
            if ara > 1.0 || kesmeler.contains(&k) || (ciplak && ara > 0.6 && son.1 - cur[0].0 >= 3.0) {
                bosalt(&mut cur, &mut out);
            }
        }
        cur.push(w);
        let sure = cur[cur.len() - 1].1 - cur[0].0;
        let t = w.2.trim();
        if t.ends_with(['.', '?', '!', '…']) && !kisaltma(t) && sure >= 0.8 {
            bosalt(&mut cur, &mut out);
        } else if (sure > 9.0 && t.ends_with([',', ';', ':'])) || sure > 14.0 {
            bosalt(&mut cur, &mut out);
        }
    }
    bosalt(&mut cur, &mut out);
    out
}

/// Türkçe hece sayısı ≈ ünlü sayısı (rakam grupları kabaca 2 hece).
pub fn hece(metin: &str) -> usize {
    let t = metin.to_lowercase();
    let unlu = t.chars().filter(|c| "aeıioöuüâîû".contains(*c)).count();
    let mut rakam = 0;
    let mut onceki = false;
    for c in t.chars() {
        let d = c.is_ascii_digit();
        if d && !onceki {
            rakam += 1;
        }
        onceki = d;
    }
    unlu + 2 * rakam
}

#[cfg(test)]
mod testler {
    use super::*;

    #[test]
    fn hece_sayisi() {
        assert_eq!(hece("Merhaba dünya"), 5);
        assert_eq!(hece("Saat 3:45"), 2 + 4);
    }

    #[test]
    fn cumle_bolme() {
        let k: Vec<Kelime> = vec![
            (0.0, 0.4, " Good".into()),
            (0.4, 0.9, " day.".into()),
            (1.0, 1.3, " Can".into()),
            (1.3, 1.5, " I".into()),
            (1.5, 2.0, " help?".into()),
        ];
        let c = cumleler(&k, &Default::default());
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].en, "Good day.");
        let c = cumleler(&k, &[3usize].into_iter().collect());
        assert_eq!(c.len(), 3);
    }
}
