//! Türkçe metnin okunuş biçimi (sayı, tarih, saat, para, birim, kısaltma, marka adı → yazıyla), ses modeline giden
//! metin için. trnorm'un (Özgür Güler, ahenk-turkish-tts, Apache-2.0 — `trnorm_LICENSE.txt`) Rust'a birebir aktarımı;
//! sözlükler `trnorm_sozluk.json`'da (Python sürümünden olduğu gibi çıkarıldı).
//!
//!   "15.00'de" → "on beşte"   "4'üncü" → "dördüncü"   "%15'lik" → "yüzde on beşlik"   "5 TL mi" → "beş lira mı"
//!
//! Python'daki karakter konumlarının yerine bayt konumları kullanılır; bağlam pencereleri karakter sayısıyla alınır.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use fancy_regex::{Captures, Regex};

type Cap<'t> = Captures<'t, str>;
use serde::Deserialize;
use unicode_normalization::UnicodeNormalization;

// ------------------------------------------------------------------ sözlük
#[derive(Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
struct Sozluk {
    abbreviations: HashMap<String, String>,
    acronyms: HashMap<String, String>,
    address_context: HashSet<String>,
    address_number_context: HashSet<String>,
    bare_number_suffixes: HashSet<String>,
    caps_words: HashSet<String>,
    code_context: Vec<String>,
    currency_subunit: HashMap<String, String>,
    currency_words: HashMap<String, String>,
    eval_acronyms: HashSet<String>,
    foreign_names: HashMap<String, String>,
    foreign_names_exact_case: HashSet<String>,
    id_context: Vec<String>,
    id_exact: HashSet<String>,
    initial_titles: Vec<String>,
    letter_names: HashMap<String, String>,
    months: Vec<String>,
    ordinal_cap_nouns: HashSet<String>,
    ordinal_cap_prefixes: Vec<String>,
    quantity_after: Vec<String>,
    quantity_before: HashSet<String>,
    score_context: Vec<String>,
    sentence_final_abbr: HashSet<String>,
    speed_units: HashMap<String, (String, String)>,
    time_context: Vec<String>,
    units: HashMap<String, String>,
}

static L: LazyLock<Sozluk> = LazyLock::new(|| serde_json::from_str(include_str!("trnorm_sozluk.json")).expect("trnorm sözlüğü"));

const LOW: &str = "a-zçğıöşüâîû";
const UP: &str = "A-ZÇĞİÖŞÜÂÎÛ";
const LET: &str = "a-zçğıöşüâîûA-ZÇĞİÖŞÜÂÎÛ";
const INT_TH: &str = r"\d{1,3}(?:\.\d{3})+";
const MARK: char = '\u{e000}';

fn rx(p: &str) -> Regex {
    Regex::new(p).unwrap_or_else(|e| panic!("trnorm düzenli ifadesi: {e}\n{p}"))
}

fn suf() -> String {
    format!("(?:'([{LET}]+))?")
}
fn num() -> String {
    format!(r"(?:{INT_TH}|\d+)(?:,\d+)?")
}
fn numu() -> String {
    format!(r"(?:{INT_TH}(?:,\d+)?|\d+[.,]\d+|\d+)")
}
const NB: &str = r"(?<![\w.,])";

fn kacis(s: &str) -> String {
    fancy_regex::escape(s).into_owned()
}

fn uzundan(keys: impl Iterator<Item = String>) -> String {
    let mut k: Vec<String> = keys.collect();
    k.sort_by(|a, b| b.chars().count().cmp(&a.chars().count()).then(a.cmp(b)));
    k.iter().map(|x| kacis(x)).collect::<Vec<_>>().join("|")
}

// ------------------------------------------------------------------ yardımcılar (büyük / küçük harf)
pub fn tr_lower(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'I' => 'ı',
            'İ' => 'i',
            c => c,
        })
        .collect::<String>()
        .to_lowercase()
}

pub fn tr_upper(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'i' => 'İ',
            'ı' => 'I',
            c => c,
        })
        .collect::<String>()
        .to_uppercase()
}

fn tr_capitalize(w: &str) -> String {
    let mut c = w.chars();
    match c.next() {
        Some(f) => tr_upper(&f.to_string()) + c.as_str(),
        None => String::new(),
    }
}

fn is_all_caps(w: &str) -> bool {
    let h: Vec<char> = w.chars().filter(|c| c.is_alphabetic()).collect();
    h.len() >= 2 && h.iter().all(|c| c.is_uppercase())
}

/// Python str.islower(): en az bir harf var ve hepsi küçük
fn is_lower(w: &str) -> bool {
    let h: Vec<char> = w.chars().filter(|c| c.is_alphabetic()).collect();
    !h.is_empty() && h.iter().all(|c| c.is_lowercase() || !c.is_uppercase())
}

/// pos baytından önceki n karakter
fn geri(t: &str, pos: usize, n: usize) -> &str {
    let bas = t[..pos].char_indices().rev().nth(n.saturating_sub(1)).map_or(0, |(i, _)| i);
    if n == 0 { &t[pos..pos] } else { &t[bas..pos] }
}

/// pos baytından sonraki n karakter
fn ileri(t: &str, pos: usize, n: usize) -> &str {
    let son = t[pos..].char_indices().nth(n).map_or(t.len(), |(i, _)| pos + i);
    &t[pos..son]
}

/// Python rx.match(text, pos): pos'ta başlayan eşleşme (^ ile kurulmuş ifadeyle)
fn eslesir_bas<'t>(r: &Regex, t: &'t str, pos: usize) -> Option<Cap<'t>> {
    r.captures(&t[pos..]).ok().flatten()
}

fn tam(r: &Regex, s: &str) -> bool {
    r.is_match(s).unwrap_or(false)
}

fn grup<'a>(c: &'a Cap, i: usize) -> Option<&'a str> {
    c.get(i).map(|m| m.as_str())
}

// ------------------------------------------------------------------ sayılar ve ek uyumu (numbers.py)
const ONES: [&str; 10] = ["sıfır", "bir", "iki", "üç", "dört", "beş", "altı", "yedi", "sekiz", "dokuz"];
const TENS: [&str; 10] = ["", "on", "yirmi", "otuz", "kırk", "elli", "altmış", "yetmiş", "seksen", "doksan"];
const SCALES: [(u128, &str); 6] =
    [(1_000_000_000_000_000_000, "kentilyon"), (1_000_000_000_000_000, "katrilyon"), (1_000_000_000_000, "trilyon"), (1_000_000_000, "milyar"), (1_000_000, "milyon"), (1000, "bin")];
const VOWELS: &str = "aeıioöuüâîû";
const BACK: &str = "aıouâû";
const ROUNDED: &str = "oöuüû";
const VOICELESS: &str = "fstkçşhp";
const FRONT_EXC: [&str; 9] = ["saat", "kontrol", "gol", "rol", "alkol", "petrol", "protokol", "hal", "kalp"];

fn unlu(c: char) -> bool {
    VOWELS.contains(c)
}

fn below_thousand(n: u128, w: &mut Vec<&'static str>) {
    let (h, r) = (n / 100, n % 100);
    if h > 0 {
        if h > 1 {
            w.push(ONES[h as usize]);
        }
        w.push("yüz");
    }
    let (t, o) = (r / 10, r % 10);
    if t > 0 {
        w.push(TENS[t as usize]);
    }
    if o > 0 {
        w.push(ONES[o as usize]);
    }
}

pub fn number_to_words(n: u128) -> String {
    if n == 0 {
        return "sıfır".into();
    }
    if n > 999_999_999_999_999_999_999 {
        return digits_to_words(&n.to_string());
    }
    let mut w = Vec::new();
    let mut n = n;
    for (v, ad) in SCALES {
        let c = n / v;
        n %= v;
        if c > 0 {
            if !(v == 1000 && c == 1) {
                below_thousand(c, &mut w);
            }
            w.push(ad);
        }
    }
    if n > 0 {
        below_thousand(n, &mut w);
    }
    w.join(" ")
}

fn sayi(s: &str) -> u128 {
    s.chars().filter(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or(0)
}

fn sayi_oku(s: &str) -> String {
    let d: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    if d.len() > 21 { digits_to_words(&d) } else { number_to_words(sayi(&d)) }
}

pub fn digits_to_words(d: &str) -> String {
    d.chars().filter_map(|c| c.to_digit(10)).map(|v| ONES[v as usize]).collect::<Vec<_>>().join(" ")
}

fn last_vowel(w: &str) -> Option<char> {
    w.chars().rev().find(|c| unlu(*c))
}

fn harmony_state(stem: &str) -> (bool, bool) {
    let last_word = stem.split_whitespace().last().unwrap_or(stem);
    if FRONT_EXC.contains(&last_word) || last_word.ends_with("saat") {
        let v = last_vowel(last_word).unwrap_or('e');
        return (false, ROUNDED.contains(v));
    }
    match last_vowel(stem) {
        None => (false, false),
        Some(v) => (BACK.contains(v), ROUNDED.contains(v)),
    }
}

fn vowel4(back: bool, rounded: bool) -> char {
    match (back, rounded) {
        (true, true) => 'u',
        (true, false) => 'ı',
        (false, true) => 'ü',
        (false, false) => 'i',
    }
}

fn vowel2(back: bool) -> char {
    if back { 'a' } else { 'e' }
}

static MNIZ: LazyLock<Regex> = LazyLock::new(|| rx("^[mn][ıiuü]z"));

pub fn harmonize_suffix(stem: &str, suffix: &str) -> String {
    if suffix.is_empty() {
        return String::new();
    }
    let mut s: Vec<char> = suffix.to_lowercase().chars().collect();
    let stem_l = stem.to_lowercase();
    let stem_last = stem_l.chars().last();
    let ends_vowel = stem_last.is_some_and(unlu);
    let ss: String = s.iter().collect();
    if !ends_vowel && s.len() >= 2 && "ysnm".contains(s[0]) {
        if s[0] == 'y' && unlu(s[1]) {
            s.remove(0);
        } else if s[0] == 's' && "ıiuü".contains(s[1]) {
            s.remove(0);
        } else if s[0] == 'n' && "ıiuü".contains(s[1]) && s.get(2) == Some(&'n') {
            s.remove(0);
        } else if ("mn".contains(s[0]) && !unlu(s[1]) && !ss.starts_with("nc")) || tam(&MNIZ, &ss) {
            s.insert(0, 'ı');
        }
    } else if !ends_vowel && (ss == "m" || ss == "n") {
        s.insert(0, 'ı');
    } else if ends_vowel && unlu(s[0]) {
        if "ıiuü".contains(s[0]) && s.get(1) == Some(&'n') && !s.get(2).is_some_and(|c| unlu(*c)) {
            s.insert(0, 'n');
        } else {
            s.insert(0, 'y');
        }
    }
    // bulunma ekinden sonraki -ki uyuma girmez ve sonrasını belirler: "2023'teki", "1990'lardakiler"
    let sabit: HashSet<usize> =
        (2..s.len().saturating_sub(1)).filter(|&i| s[i] == 'k' && "ıi".contains(s[i + 1]) && "dt".contains(s[i - 2]) && "ae".contains(s[i - 1])).map(|i| i + 1).collect();
    let (mut back, mut rounded) = harmony_state(&stem_l);
    let mut prev = stem_last.unwrap_or(' ');
    let mut out = String::new();
    for (i, &c0) in s.iter().enumerate() {
        let mut c = c0;
        if sabit.contains(&i) {
            c = 'i';
            back = false;
            rounded = false;
        } else if "aeâ".contains(c) {
            c = vowel2(back);
            back = BACK.contains(c);
            rounded = false;
        } else if "ıiuü".contains(c) {
            c = vowel4(back, rounded);
            rounded = ROUNDED.contains(c);
        } else if "oö".contains(c) {
            back = c == 'o';
            rounded = true;
        } else if "dt".contains(c) && i == 0 {
            c = if VOICELESS.contains(prev) { 't' } else { 'd' };
        } else if "cç".contains(c) && i == 0 {
            c = if VOICELESS.contains(prev) { 'ç' } else { 'c' };
        }
        out.push(c);
        prev = c;
    }
    out
}

fn harmonize_clitic(stem: &str, clitic: &str) -> String {
    let (mut back, mut rounded) = harmony_state(stem);
    clitic
        .chars()
        .map(|c| {
            if "ae".contains(c) {
                let v = vowel2(back);
                back = BACK.contains(v);
                rounded = false;
                v
            } else if "ıiuü".contains(c) {
                let v = vowel4(back, rounded);
                rounded = ROUNDED.contains(v);
                v
            } else {
                c
            }
        })
        .collect()
}

pub fn attach_suffix(stem: &str, suffix: &str) -> String {
    if suffix.is_empty() {
        return stem.to_string();
    }
    let suf = harmonize_suffix(stem, suffix);
    let mut stem = stem.to_string();
    if suf.chars().next().is_some_and(unlu) && stem.ends_with("dört") {
        stem = stem[..stem.len() - "dört".len()].to_string() + "dörd";
    }
    stem + &suf
}

pub fn ordinal_to_words(n: u128) -> String {
    let mut stem = number_to_words(n);
    let (b, r) = harmony_state(&stem);
    let v = vowel4(b, r);
    let suf = if stem.chars().last().is_some_and(unlu) { format!("nc{v}") } else { format!("{v}nc{v}") };
    if suf.chars().next().is_some_and(unlu) && stem.ends_with("dört") {
        stem = stem[..stem.len() - "dört".len()].to_string() + "dörd";
    }
    stem + &suf
}

fn read_decimal_fraction(d: &str) -> String {
    if d.is_empty() {
        return String::new();
    }
    if d.chars().count() > 3 {
        return digits_to_words(d);
    }
    read_digit_group(d)
}

fn read_digit_group(g: &str) -> String {
    let rest = g.trim_start_matches('0');
    let zeros = g.len() - rest.len();
    let mut p: Vec<String> = vec!["sıfır".to_string(); zeros];
    if !rest.is_empty() {
        p.push(sayi_oku(rest));
    }
    p.join(" ")
}

fn read_long_digits(d: &str) -> String {
    let n = d.len();
    if n <= 4 {
        return read_digit_group(d);
    }
    let sizes: Vec<usize> = match n {
        5 => vec![3, 2],
        6 => vec![3, 3],
        7 => vec![3, 2, 2],
        8 => vec![3, 3, 2],
        9 => vec![3, 3, 3],
        10 => vec![3, 3, 2, 2],
        11 => vec![3, 3, 3, 2],
        _ => {
            let mut s = vec![3; n / 3];
            if n % 3 == 1 {
                *s.last_mut().unwrap() = 2;
                s.push(2);
            } else if n % 3 == 2 {
                s.push(2);
            }
            s
        }
    };
    let mut i = 0;
    let mut g = Vec::new();
    for z in sizes {
        g.push(read_digit_group(&d[i..i + z]));
        i += z;
    }
    g.join(", ")
}

// ------------------------------------------------------------------ normalizer yardımcıları
static ORDINAL_SUFFIX: LazyLock<Regex> = LazyLock::new(|| rx("^(?:[ıiuü]?nc[ıiuü])(.*)$"));
static INT_TH_TAM: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"^{INT_TH}$")));

fn ekle(stem: &str, suffix: Option<&str>) -> String {
    match suffix {
        Some(s) if !s.is_empty() => attach_suffix(stem, &tr_lower(s)),
        _ => stem.to_string(),
    }
}

fn sayi_eki(n: u128, words: &str, suffix: Option<&str>) -> String {
    let Some(s) = suffix.filter(|s| !s.is_empty()) else { return words.to_string() };
    let s = tr_lower(s);
    if let Ok(Some(m)) = ORDINAL_SUFFIX.captures(s.as_str()) {
        let kalan = grup(&m, 1).unwrap_or("");
        return if kalan.is_empty() { ordinal_to_words(n) } else { ekle(&ordinal_to_words(n), Some(kalan)) };
    }
    attach_suffix(words, &s)
}

pub fn read_number(s: &str, dot_word: &str) -> String {
    let s = s.trim();
    let neg = s.starts_with('-') || s.starts_with('−');
    let s = if neg { &s[s.char_indices().nth(1).map_or(s.len(), |x| x.0)..] } else { s };
    let out = if let Some((ip, fp)) = s.split_once(',') {
        let ipv = sayi(&ip.replace('.', ""));
        if fp == "5" && ipv > 0 { format!("{} buçuk", number_to_words(ipv)) } else { format!("{} virgül {}", number_to_words(ipv), read_decimal_fraction(fp)) }
    } else if s.contains('.') {
        if tam(&INT_TH_TAM, s) {
            number_to_words(sayi(&s.replace('.', "")))
        } else {
            let (ip, fp) = s.split_once('.').unwrap();
            if dot_word == "virgül" && fp == "5" && sayi(ip) > 0 {
                format!("{} buçuk", number_to_words(sayi(ip)))
            } else {
                format!("{} {dot_word} {}", number_to_words(sayi(ip)), read_decimal_fraction(&fp.replace('.', "")))
            }
        }
    } else {
        sayi_oku(s)
    };
    if neg { format!("eksi {out}") } else { out }
}

fn int_value(s: &str) -> Option<u128> {
    let s = s.replace('.', "");
    (!s.is_empty() && s.chars().all(|c| c.is_ascii_digit()) && s.len() <= 30).then(|| sayi(&s))
}

static BETWEEN: LazyLock<Regex> = LazyLock::new(|| rx(r"^[ \t]+(?:arası|arasında|arasındaki|aralığı|aralığında)\b"));

fn ila(text: &str, end: usize) -> &'static str {
    if eslesir_bas(&BETWEEN, text, end).is_some() { "ile" } else { "ila" }
}

fn read_range(a: &str, b: &str, dot: &str, word: &str) -> String {
    format!("{} {word} {}", read_number(a, dot), read_number(b, dot))
}

fn harfler(tok: &str) -> String {
    let h: Vec<char> = tok.chars().filter(|c| c.is_alphabetic()).collect();
    h.iter()
        .map(|&c| if c == 'I' && h.len() > 1 { "i".to_string() } else { L.letter_names.get(&c.to_string()).cloned().unwrap_or_else(|| tr_lower(&c.to_string())) })
        .collect::<Vec<_>>()
        .join(" ")
}

fn pronounceable(w: &str) -> bool {
    let f: Vec<bool> = tr_lower(w).chars().filter(|c| c.is_alphabetic()).map(unlu).collect();
    let n = f.len();
    if n <= 2 || !f.iter().any(|&v| v) {
        return false;
    }
    let (mut run, mut maxrun) = (0, 0);
    for &v in &f {
        run = if v { 0 } else { run + 1 };
        maxrun = maxrun.max(run);
    }
    if n >= 5 {
        return maxrun <= 3 && f.iter().filter(|&&v| v).count() as f64 / n as f64 >= 0.3;
    }
    let starts_cc = !f[0] && !f[1];
    let ends_cc = !f[n - 1] && !f[n - 2];
    maxrun <= 2 && !starts_cc && !ends_cc
}

// ------------------------------------------------------------------ mI / dA ekleri
static CLITIC: LazyLock<Regex> = LazyLock::new(|| {
    let (i4, a2) = ("[ıiuü]", "[ae]");
    let pers = format!("(?:m|n(?:{i4}z)?|k|l{a2}r)");
    rx(&format!(
        "^(?:d{a2}|m{i4}(?:y{i4}[mz]|s{i4}n(?:{i4}z)?|d{i4}r(?:l{a2}r)?|yd{i4}{pers}?|ym{i4}ş(?:{i4}[mz]|s{i4}n(?:{i4}z)?|l{a2}r)?|ys{a2}{pers}?)?)$"
    ))
});
static WORD_AFTER: LazyLock<Regex> = LazyLock::new(|| rx(&format!("^([ \\t]+)([{LET}]+)(?![\\w'])")));
static QUESTION_NEXT: LazyLock<Regex> = LazyLock::new(|| rx("^\\s*[\"'’”»)\\]]*\\s*\\?"));
static BARE_COUNT: LazyLock<Regex> = LazyLock::new(|| rx(r"^[\d.,\s-]+$"));

/// pos'ta " <ek>" (mI / dA biçimleri, tümü küçük ya da tümü büyük) → (boşluk, kelime, bitiş)
fn clitic_at(text: &str, pos: usize, count: bool) -> Option<(String, String, usize)> {
    let m = eslesir_bas(&WORD_AFTER, text, pos)?;
    let (bos, kelime) = (grup(&m, 1)?.to_string(), grup(&m, 2)?.to_string());
    let son = pos + m.get(0)?.end();
    let low = tr_lower(&kelime);
    if !(kelime == low || kelime == tr_upper(&low)) || !tam(&CLITIC, &low) {
        return None;
    }
    if low.starts_with("müdür") && (count || eslesir_bas(&QUESTION_NEXT, text, son).is_none()) {
        return None;
    }
    Some((bos, kelime, son))
}

fn last_word(s: &str) -> String {
    s.split_whitespace().last().map(tr_lower).unwrap_or_default()
}

/// rx ile değiştir; ardından mI / dA gelen açılımları işaretle (sonra ek uyumu için)
fn sub(r: &Regex, text: &str, mut f: impl FnMut(&Cap) -> String) -> String {
    let mut out = String::new();
    let mut pos = 0;
    for c in r.captures_iter(text) {
        let Ok(c) = c else { break };
        let m = c.get(0).unwrap();
        let yeni = f(&c);
        out.push_str(&text[pos..m.start()]);
        out.push_str(&yeni);
        let count = tam(&BARE_COUNT, m.as_str());
        let lw = last_word(&yeni);
        if !lw.is_empty() && lw != last_word(m.as_str()) && clitic_at(text, m.end(), count).is_some() {
            out.push(MARK);
        }
        pos = m.end();
    }
    out.push_str(&text[pos..]);
    out
}

/// Düz değiştirme (işaretsiz), Python re.sub(fonksiyon)
fn degistir(r: &Regex, text: &str, mut f: impl FnMut(&Cap) -> String) -> String {
    let mut out = String::new();
    let mut pos = 0;
    for c in r.captures_iter(text) {
        let Ok(c) = c else { break };
        let m = c.get(0).unwrap();
        out.push_str(&text[pos..m.start()]);
        out.push_str(&f(&c));
        pos = m.end();
    }
    out.push_str(&text[pos..]);
    out
}

fn pass_clitics(text: &str) -> String {
    let mut parcalar = text.split(MARK);
    let mut out = parcalar.next().unwrap_or("").to_string();
    for part in parcalar {
        let (mut pos, mut prev) = (0, String::new());
        while let Some((bos, kelime, son)) = clitic_at(part, pos, false) {
            if !(prev.is_empty() || ((prev == "da" || prev == "de") && tr_lower(&kelime).starts_with('m'))) {
                break;
            }
            let bag = format!("{}{bos}", geri(&out, out.len(), 48));
            prev = harmonize_clitic(&tr_lower(&bag), &tr_lower(&kelime));
            out.push_str(&bos);
            out.push_str(&if kelime != tr_lower(&kelime) { tr_upper(&prev) } else { prev.clone() });
            pos = son;
        }
        out.push_str(&part[pos..]);
    }
    out
}

fn context_before(text: &str, pos: usize, width: usize) -> String {
    tr_lower(geri(text, pos, width))
}

static RAKAM: LazyLock<Regex> = LazyLock::new(|| rx(r"\d"));
static KUCUK_KELIME: LazyLock<Regex> = LazyLock::new(|| rx(&format!("[{LOW}]+")));
static MONTHS_LOWER: LazyLock<HashSet<String>> = LazyLock::new(|| L.months.iter().filter(|m| !m.is_empty()).map(|m| tr_lower(m)).collect());

fn son_parca(ctx: &str) -> &str {
    let mut son = 0;
    for m in RAKAM.find_iter(ctx).flatten() {
        son = m.end();
    }
    &ctx[son..]
}

fn kucuk_kelimeler(s: &str) -> Vec<String> {
    KUCUK_KELIME.find_iter(s).flatten().map(|m| m.as_str().to_string()).collect()
}

fn has_word(ctx: &str, words: &[String], last: usize) -> bool {
    let toks = kucuk_kelimeler(son_parca(ctx));
    if toks.last().is_some_and(|t| MONTHS_LOWER.contains(t)) {
        return false;
    }
    toks[toks.len().saturating_sub(last)..].iter().any(|t| words.iter().any(|w| t.starts_with(w.as_str())))
}

static NOMINAL_SUFFIX: LazyLock<Regex> = LazyLock::new(|| {
    rx(r"^(?:[ıiuü]|s[ıiuü]|[ıiuü]?m(?:[ıiuü]z)?|[ıiuü]?n(?:[ıiuü]z)?|l[ae]r(?:[ıi](?:m[ıi]z|n[ıi]z|[mn])?)?|l[ıiuü]k?|s[ıiuü]z)?(?:n?[ıiuü]n|y?[ae]|n[ae]|n?[dt][ae]n?|y?l[ae]|y?[ıiuü]|n[ıiuü])?$")
});

fn cue_word(tok: &str, cues: &[String]) -> bool {
    cues.iter().any(|c| tok == c || (!L.id_exact.contains(c) && tok.starts_with(c.as_str()) && tam(&NOMINAL_SUFFIX, &tok[c.len()..])))
}

fn has_cue(ctx: &str, cues: &[String], last: usize) -> bool {
    let toks = kucuk_kelimeler(son_parca(ctx));
    if toks.last().is_some_and(|t| MONTHS_LOWER.contains(t)) {
        return false;
    }
    toks[toks.len().saturating_sub(last)..].iter().any(|t| cue_word(t, cues))
}

static ILK_KUCUK: LazyLock<Regex> = LazyLock::new(|| rx(&format!("^[ \\t]*([{LOW}]+)")));

fn quantity(ctx: &str, after: &str) -> bool {
    let toks = kucuk_kelimeler(son_parca(ctx));
    if toks.last().is_some_and(|t| L.quantity_before.contains(t)) {
        return true;
    }
    eslesir_bas(&ILK_KUCUK, after, 0).and_then(|m| grup(&m, 1).map(|w| cue_word(w, &L.quantity_after))).unwrap_or(false)
}

static UNIT_WORDS: LazyLock<String> = LazyLock::new(|| {
    let mut s: HashSet<String> = L.units.values().cloned().collect();
    for w in ["kilo", "lira", "dolar", "avro", "kuruş", "derece", "saat", "dakika", "saniye", "gün", "yıl", "ay", "hafta"] {
        s.insert(w.into());
    }
    uzundan(s.into_iter())
});

// ------------------------------------------------------------------ geçişler
static EMOJI: LazyLock<Regex> = LazyLock::new(|| rx("[\u{1F000}-\u{1FAFF}\u{1FC00}-\u{1FFFD}\u{2600}-\u{27BF}\u{2B00}-\u{2BFF}\u{2300}-\u{23FF}\u{FE0E}\u{FE0F}\u{20E3}]+"));
static P_KESME: LazyLock<Regex> = LazyLock::new(|| rx("(?<=\\w)[\u{2019}\u{2018}`\u{b4}\u{2bc}\u{2032}](?=\\w)"));
static P_TIRNAK: LazyLock<Regex> = LazyLock::new(|| rx("[\u{201c}\u{201d}\u{201e}\u{ab}\u{bb}]"));
static P_TEK: LazyLock<Regex> = LazyLock::new(|| rx("[\u{2019}\u{2018}]"));
static P_TIRE_SAYI: LazyLock<Regex> = LazyLock::new(|| rx("(?<=\\d)\\s?[\u{2013}\u{2014}]\\s?(?=\\d)"));
static P_TIRE: LazyLock<Regex> = LazyLock::new(|| rx("\\s[\u{2013}\u{2014}]\\s"));
static P_BOSLUK: LazyLock<Regex> = LazyLock::new(|| rx("[ \\t]+"));

fn preclean(t: &str) -> String {
    let t: String = t.nfc().filter(|&c| c != MARK).collect();
    let t = EMOJI.replace_all(&t, " ").into_owned();
    let t: String = t.chars().filter(|c| !['\u{200b}', '\u{200c}', '\u{200d}', '\u{feff}'].contains(c)).collect();
    let t = t.replace(['\u{a0}', '\u{202f}'], " ");
    let t = P_KESME.replace_all(&t, "'").into_owned();
    let t = P_TIRNAK.replace_all(&t, "\"").into_owned();
    let t = P_TEK.replace_all(&t, "'").into_owned();
    let t = t.replace('…', "...");
    let t = P_TIRE_SAYI.replace_all(&t, "-").into_owned();
    let t = P_TIRE.replace_all(&t, ", ").into_owned();
    let t = t.replace('−', "-");
    P_BOSLUK.replace_all(&t, " ").trim().to_string()
}

// --- yabancı adlar / markalar
static NAMES: LazyLock<(HashMap<String, String>, Regex)> = LazyLock::new(|| {
    let mut tablo: HashMap<String, String> = HashMap::new();
    let mut anahtarlar: Vec<&String> = L.foreign_names.keys().collect();
    anahtarlar.sort();
    for k in &anahtarlar {
        let sp = &L.foreign_names[*k];
        let bicim: Vec<String> = if L.foreign_names_exact_case.contains(*k) {
            vec![k.to_string(), k.to_uppercase()]
        } else {
            let low = k.to_lowercase();
            let mut ilk = low.chars();
            let f = ilk.next().map(|c| c.to_string()).unwrap_or_default();
            let rest = ilk.as_str().to_string();
            vec![k.to_string(), k.to_uppercase(), tr_upper(k), low.clone(), f.to_uppercase() + &rest, tr_upper(&f) + &rest]
        };
        for b in bicim {
            tablo.entry(b).or_insert_with(|| sp.clone());
        }
    }
    for k in &anahtarlar {
        tablo.insert(k.to_string(), L.foreign_names[*k].clone());
    }
    let alt = uzundan(tablo.keys().cloned());
    let r = rx(&format!("(?<![\\w@./:\\\\])(?<!\\w')({alt})(?:'([{LET}]+))?(?![\\w@+]|'[{LET}]|[./:]\\w)"));
    (tablo, r)
});

fn pass_names(t: &str) -> String {
    sub(&NAMES.1, t, |m| {
        let yazilan = grup(m, 1).unwrap();
        let sp = ekle(&NAMES.0[yazilan], grup(m, 2));
        if yazilan.chars().next().is_some_and(|c| c.is_uppercase()) && !is_all_caps(yazilan) { tr_capitalize(&sp) } else { sp }
    })
}

// --- web
static URL: LazyLock<Regex> = LazyLock::new(|| rx(r"(?i)\b(?:https?://)?(?:www\.)?[\w-]+(?:\.[\w-]+)*\.(?:com|net|org|gov|edu|io|ai|tr|co|info|biz|me|app|dev)(?:\.[a-z]{2})?(?:/[^\s]*)?"));
static EMAIL: LazyLock<Regex> = LazyLock::new(|| rx(r"\b[\w.+-]+@[\w-]+(?:\.[\w-]+)+\b"));
static HTTP_BAS: LazyLock<Regex> = LazyLock::new(|| rx(r"(?i)^https?://"));

fn read_web_token(tok: &str) -> String {
    let mut p = Vec::new();
    let mut parca = String::new();
    let mut parcalar = Vec::new();
    for c in tok.chars() {
        if "./@_-".contains(c) {
            parcalar.push(std::mem::take(&mut parca));
            parcalar.push(c.to_string());
        } else {
            parca.push(c);
        }
    }
    parcalar.push(parca);
    for x in parcalar.into_iter().filter(|x| !x.is_empty()) {
        let low = x.to_lowercase();
        p.push(match x.as_str() {
            "." => "nokta".into(),
            "/" => "eğik çizgi".into(),
            "@" => "et".into(),
            "_" => "alt çizgi".into(),
            "-" => "tire".into(),
            _ if low == "www" => "ve ve ve".into(),
            _ if ["com", "tr", "net", "org", "gov", "edu", "io", "ai"].contains(&low.as_str()) => {
                match low.as_str() {
                    "com" => "kom",
                    "tr" => "te re",
                    "io" => "ay o",
                    "ai" => "ey ay",
                    o => o,
                }
                .to_string()
            }
            _ if x.chars().all(|c| c.is_ascii_digit()) => {
                if x.len() <= 4 { read_digit_group(&x) } else { digits_to_words(&x) }
            }
            _ => tr_lower(&x),
        });
    }
    p.join(" ")
}

fn pass_web(t: &str) -> String {
    let t = sub(&EMAIL, t, |m| read_web_token(m.get(0).unwrap().as_str()));
    sub(&URL, &t, |m| {
        let tok = HTTP_BAS.replace(m.get(0).unwrap().as_str(), "").into_owned();
        read_web_token(tok.trim_end_matches('/'))
    })
}

// --- kısaltmalar
static ABBR: LazyLock<Regex> = LazyLock::new(|| rx(&format!("(?<![\\w.])({})(?=[\\s,;:!?)\"']|$)", uzundan(L.abbreviations.keys().cloned()))));
static NO_RE: LazyLock<Regex> = LazyLock::new(|| rx(r"(?<![\w.])(No|Nu|NO)\s?[:.]\s?(?=\d)"));
static KAT_RE: LazyLock<Regex> = LazyLock::new(|| rx(r"(?<![\w.])(Kat|Daire|Blok|D|Tel|GSM)\s?:\s?(?=\d)"));
static PAGE: LazyLock<Regex> = LazyLock::new(|| rx(r"(?<![\w.])([sc])\.\s?(?=\d)"));
static BUYUK_SONRA: LazyLock<Regex> = LazyLock::new(|| rx(&format!("^\\s+[{UP}]")));

fn pass_abbreviations(t: &str) -> String {
    let metin = t.to_string();
    let t = sub(&ABBR, t, |m| {
        let k = grup(m, 1).unwrap();
        let mut out = L.abbreviations[k].clone();
        if L.sentence_final_abbr.contains(k) {
            let rest = &metin[m.get(0).unwrap().end()..];
            if rest.trim().is_empty() || eslesir_bas(&BUYUK_SONRA, rest, 0).is_some() {
                out.push('.');
            }
        }
        out
    });
    let t = degistir(&NO_RE, &t, |_| "numara ".into());
    let t = degistir(&KAT_RE, &t, |m| {
        let k = grup(m, 1).unwrap();
        format!("{} ", match k {
            "D" => "daire",
            "Tel" => "telefon",
            "GSM" => "ge se me",
            o => o,
        })
    });
    degistir(&PAGE, &t, |m| if grup(m, 1) == Some("s") { "sayfa ".into() } else { "cilt ".into() })
}

static PRODUCT: LazyLock<Regex> = LazyLock::new(|| rx(r"(?<=\d)[ \t]?[x×*][ \t]?(?=[\d₺$€£])"));

fn pass_products(t: &str) -> String {
    degistir(&PRODUCT, t, |m| {
        let s = m.get(0).unwrap().start();
        let once: Vec<char> = t[..s].chars().rev().take(2).collect();
        if m.get(0).unwrap().as_str().trim() == "x" && once.first() == Some(&'0') && once.get(1).is_none_or(|c| !c.is_alphanumeric()) {
            m.get(0).unwrap().as_str().to_string() // onaltılık 0x1F
        } else {
            " çarpı ".into()
        }
    })
}

static PHONE: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"(?<![\w+])(\+90[\s.-]?|0[\s.-]?)?\(?([2-58]\d{{2}})\)?[\s.-]?(\d{{3}})[\s.-]?(\d{{2}})[\s.-]?(\d{{2}}){}(?![\w])", suf())));
static PHONE444: LazyLock<Regex> = LazyLock::new(|| rx(r"(?<![\w])(444)[\s.-]?(\d)[\s.-]?(\d{3})(?!\d)"));
static AYIRAC: LazyLock<Regex> = LazyLock::new(|| rx(r"[\s.()-]"));

fn pass_phones(t: &str) -> String {
    let t = sub(&PHONE, t, |m| {
        let ham = m.get(0).unwrap().as_str();
        let on = grup(m, 1);
        if on.is_none() && !AYIRAC.is_match(ham).unwrap_or(false) {
            return ham.to_string();
        }
        let alan = number_to_words(sayi(grup(m, 2).unwrap()));
        let mut p = vec![match on {
            Some(o) if o.starts_with('+') => format!("artı doksan, {alan}"),
            Some(_) => format!("sıfır {alan}"),
            None => alan,
        }];
        p.push(read_digit_group(grup(m, 3).unwrap()));
        p.push(read_digit_group(grup(m, 4).unwrap()));
        p.push(ekle(&read_digit_group(grup(m, 5).unwrap()), grup(m, 6)));
        p.join(", ")
    });
    sub(&PHONE444, &t, |m| format!("dört yüz kırk dört, {}, {}", read_digit_group(grup(m, 2).unwrap()), read_digit_group(grup(m, 3).unwrap())))
}

static IBAN: LazyLock<Regex> = LazyLock::new(|| rx(r"\bTR\s?(\d{2})((?:\s?\d{4}){5})\s?(\d{2})\b"));
static DORTLU: LazyLock<Regex> = LazyLock::new(|| rx(r"\d{4}"));

fn pass_iban(t: &str) -> String {
    sub(&IBAN, t, |m| {
        let mut g = vec![read_digit_group(grup(m, 1).unwrap())];
        g.extend(DORTLU.find_iter(grup(m, 2).unwrap()).flatten().map(|x| digits_to_words(x.as_str())));
        g.push(read_digit_group(grup(m, 3).unwrap()));
        format!("te re {}", g.join(", "))
    })
}

static PLATE: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"(?<![\w])(0[1-9]|[1-7]\d|8[01])\s([{UP}]{{1,3}})\s(\d{{2,5}}){}(?![\w])", suf())));

fn pass_plates(t: &str) -> String {
    sub(&PLATE, t, |m| {
        let n = grup(m, 3).unwrap();
        let kuyruk = if n.len() == 4 { format!("{} {}", read_digit_group(&n[..2]), read_digit_group(&n[2..])) } else { read_digit_group(n) };
        ekle(&format!("{} {} {kuyruk}", read_digit_group(grup(m, 1).unwrap()), harfler(grup(m, 2).unwrap())), grup(m, 4))
    })
}

// --- tarih, saat
fn date_words(d: u128, mo: usize, y: Option<&str>) -> String {
    let mut o = format!("{} {}", number_to_words(d), L.months[mo]);
    if let Some(y) = y {
        o += &format!(" {}", number_to_words(sayi(y)));
    }
    o
}

static DATE: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"{NB}(\d{{1,2}})([./-])(\d{{1,2}})\2(\d{{4}}|\d{{2}}){}(?![\w]|[.,]\d)", suf())));
static MONTH_YEAR: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"{NB}(0?[1-9]|1[0-2])[./]((?:19|20)\d{{2}}){}(?![\w]|[.,]\d)", suf())));
static ISO_DATE: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"{NB}(\d{{4}})-(\d{{2}})-(\d{{2}}){}(?![\w]|[.,]\d)", suf())));
static YMD: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"{NB}((?:1[89]|20)\d{{2}})(?:/(\d{{1,2}})/(\d{{1,2}})|\.(\d{{2}})\.(\d{{2}})){}(?![\w]|[.,/]\d)", suf())));

fn pass_dates(t: &str) -> String {
    let t = sub(&YMD, t, |m| {
        let mo = sayi(grup(m, 2).or(grup(m, 4)).unwrap_or("0"));
        let d = sayi(grup(m, 3).or(grup(m, 5)).unwrap_or("0"));
        if (1..=12).contains(&mo) && (1..=31).contains(&d) { ekle(&date_words(d, mo as usize, grup(m, 1)), grup(m, 6)) } else { m.get(0).unwrap().as_str().into() }
    });
    let t = sub(&ISO_DATE, &t, |m| {
        let (mo, d) = (sayi(grup(m, 2).unwrap()), sayi(grup(m, 3).unwrap()));
        if (1..=12).contains(&mo) && (1..=31).contains(&d) { ekle(&date_words(d, mo as usize, grup(m, 1)), grup(m, 4)) } else { m.get(0).unwrap().as_str().into() }
    });
    let t = sub(&DATE, &t, |m| {
        let (d, mo) = (sayi(grup(m, 1).unwrap()), sayi(grup(m, 3).unwrap()));
        if (1..=12).contains(&mo) && (1..=31).contains(&d) { ekle(&date_words(d, mo as usize, grup(m, 4)), grup(m, 5)) } else { m.get(0).unwrap().as_str().into() }
    });
    sub(&MONTH_YEAR, &t, |m| ekle(&format!("{} {}", L.months[sayi(grup(m, 1).unwrap()) as usize], number_to_words(sayi(grup(m, 2).unwrap()))), grup(m, 3)))
}

fn time_words(h: &str, mi: &str, s: Option<&str>) -> String {
    let mut o = number_to_words(sayi(h));
    if let Some(s) = s {
        return format!("{o} {} {}", read_digit_group(mi), read_digit_group(s));
    }
    if mi != "00" {
        o += &format!(" {}", read_digit_group(mi));
    }
    o
}

static TIME_COLON: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"{NB}([01]?\d|2[0-4]):([0-5]\d)(?::([0-5]\d))?{}(?![\w]|[.,]\d)", suf())));
static TIME_DOT: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"{NB}([01]?\d|2[0-4])\.([0-5]\d){}(?![\w]|[.,]\d)", suf())));
static TIME_RANGE: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"{NB}([01]?\d|2[0-4])[:.]([0-5]\d)\s?-\s?([01]?\d|2[0-4])[:.]([0-5]\d){}(?![\w]|[.,]\d)", suf())));
static PARA_ONCE: LazyLock<Regex> = LazyLock::new(|| rx(r"[%‰₺$€£]\s?$"));
static ZAMAN_SONRA: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"^\s+(?:{})", L.time_context.join("|"))));

fn pass_times(t: &str) -> String {
    let metin = t.to_string();
    let t = sub(&TIME_RANGE, t, |m| {
        let e = m.get(0).unwrap().end();
        ekle(&format!("{} {} {}", time_words(grup(m, 1).unwrap(), grup(m, 2).unwrap(), None), ila(&metin, e), time_words(grup(m, 3).unwrap(), grup(m, 4).unwrap(), None)), grup(m, 5))
    });
    let t = sub(&TIME_COLON, &t, |m| ekle(&time_words(grup(m, 1).unwrap(), grup(m, 2).unwrap(), grup(m, 3)), grup(m, 4)));
    let metin = t.clone();
    sub(&TIME_DOT, &t, |m| {
        let (h, mi, sf) = (grup(m, 1).unwrap(), grup(m, 2).unwrap(), grup(m, 3));
        let (s, e) = (m.get(0).unwrap().start(), m.get(0).unwrap().end());
        if PARA_ONCE.is_match(geri(&metin, s, 2)).unwrap_or(false) {
            return m.get(0).unwrap().as_str().into();
        }
        let once = context_before(&metin, s, 32);
        let sonra = tr_lower(ileri(&metin, e, 16));
        let zamanli = sf.is_some() || has_word(&once, &L.time_context, 3) || mi == "00" || mi == "30" || eslesir_bas(&ZAMAN_SONRA, &sonra, 0).is_some();
        if !zamanli {
            return m.get(0).unwrap().as_str().into();
        }
        ekle(&time_words(h, mi, None), sf)
    })
}

// --- para, yüzde, derece, birim
const CUR_SYMBOLS: &str = r"₺|\$|€|£|¥|₽";
const CUR_CODES: &str = "TL|TRY|USD|EUR|GBP|JPY|RUB|CHF";
static CUR_PREFIX: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"(?<![\w])({CUR_SYMBOLS})\s?({}){}(?![\w]|[.,]\d)", numu(), suf())));
static CUR_POSTFIX: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"{NB}({0})(?:\s?-\s?({0}))?\s?({CUR_SYMBOLS}|(?:{CUR_CODES})(?![\w])){1}(?![\w]|[.,]\d)", numu(), suf())));
static SCALE_CUR: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"\b(bin|milyon|milyar|trilyon)\s({CUR_SYMBOLS}|{CUR_CODES})(?![\w]){}", suf())));
static KURUSLU: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"^({INT_TH}|\d+)[.,](\d{{2}})$")));

fn money_words(amount: &str, cur: &str) -> String {
    if let (Some(alt), Ok(Some(m))) = (L.currency_subunit.get(cur), KURUSLU.captures(amount)) {
        if !tam(&INT_TH_TAM, amount) {
            let major = number_to_words(sayi(&grup(&m, 1).unwrap().replace('.', "")));
            let minor = sayi(grup(&m, 2).unwrap());
            return if minor > 0 { format!("{major} {cur} {} {alt}", number_to_words(minor)) } else { format!("{major} {cur}") };
        }
    }
    format!("{} {cur}", read_number(amount, "virgül"))
}

fn pass_money(t: &str) -> String {
    let t = sub(&CUR_PREFIX, t, |m| ekle(&money_words(grup(m, 2).unwrap(), &L.currency_words[grup(m, 1).unwrap()]), grup(m, 3)));
    let metin = t.clone();
    let t = sub(&CUR_POSTFIX, &t, |m| {
        let w = &L.currency_words[grup(m, 3).unwrap()];
        match grup(m, 2) {
            Some(b) => ekle(&format!("{} {} {}", read_number(grup(m, 1).unwrap(), "virgül"), ila(&metin, m.get(0).unwrap().end()), money_words(b, w)), grup(m, 4)),
            None => ekle(&money_words(grup(m, 1).unwrap(), w), grup(m, 4)),
        }
    });
    sub(&SCALE_CUR, &t, |m| ekle(&format!("{} {}", grup(m, 1).unwrap(), L.currency_words[grup(m, 2).unwrap()]), grup(m, 3)))
}

static PCT_PREFIX: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"(?<![\w])([%‰])\s?({0})(?:\s?-\s?%?\s?({0}))?{1}(?![\w]|[.,]\d)", numu(), suf())));
static PCT_POSTFIX: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"{NB}({})\s?%{}(?![\w])", numu(), suf())));

fn pass_percent(t: &str) -> String {
    let metin = t.to_string();
    let t = sub(&PCT_PREFIX, t, |m| {
        let w = if grup(m, 1) == Some("%") { "yüzde" } else { "binde" };
        let a = grup(m, 2).unwrap();
        let body = match grup(m, 3) {
            None => read_number(a, "virgül"),
            Some(b) => read_range(a, b, "virgül", ila(&metin, m.get(0).unwrap().end())),
        };
        ekle(&format!("{w} {body}"), grup(m, 4))
    });
    sub(&PCT_POSTFIX, &t, |m| ekle(&format!("yüzde {}", read_number(grup(m, 1).unwrap(), "virgül")), grup(m, 2)))
}

static DEG: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"(?<![\w.,])(-?{})\s?°\s?([CF])?(?![{LET}]){}", numu(), suf())));

fn pass_degrees(t: &str) -> String {
    sub(&DEG, t, |m| {
        let mut o = format!("{} derece", read_number(grup(m, 1).unwrap(), "virgül"));
        if grup(m, 2) == Some("F") {
            o += " fahrenhayt";
        }
        ekle(&o, grup(m, 3))
    })
}

static UNIT: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"{NB}(-?{0})(?:\s?-\s?({0}))?\s?({1})(?![\w²³/]){2}", numu(), uzundan(L.units.keys().cloned()), suf())));
static SPEED: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"{NB}({0})(?:\s?-\s?({0}))?\s?({1})(?![\w]){2}", numu(), uzundan(L.speed_units.keys().cloned()), suf())));

fn pass_units(t: &str) -> String {
    let metin = t.to_string();
    let t = sub(&SPEED, t, |m| {
        let (per, w) = &L.speed_units[grup(m, 3).unwrap()];
        let a = grup(m, 1).unwrap();
        let n = match grup(m, 2) {
            None => read_number(a, "virgül"),
            Some(b) => read_range(a, b, "virgül", ila(&metin, m.get(0).unwrap().end())),
        };
        ekle(&format!("{per} {n} {w}"), grup(m, 4))
    });
    let metin = t.clone();
    sub(&UNIT, &t, |m| {
        let a = grup(m, 1).unwrap();
        let n = match grup(m, 2) {
            None => read_number(a, "virgül"),
            Some(b) => read_range(a, b, "virgül", ila(&metin, m.get(0).unwrap().end())),
        };
        ekle(&format!("{n} {}", L.units[grup(m, 3).unwrap()]), grup(m, 4))
    })
}

// --- harf + rakam karışık, aralık, kesir, Roma rakamı, sıra sayısı
static ALNUM: LazyLock<Regex> = LazyLock::new(|| {
    rx(&format!(r"(?<![\w'])(?=[{LET}\d-]*\d)(?=[{LET}\d-]*[{UP}])([{LET}\d]+(?:-[{LET}\d]+)*(?:(?<=\d)[.,]\d+)?){}(?![\w]|[.,]\d)", suf()))
});
static ALNUM_PARCA: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"[{LET}]+|\d+(?:[.,]\d+)?")));

fn read_alnum(tok: &str) -> String {
    let mut o = Vec::new();
    for p in ALNUM_PARCA.find_iter(tok).flatten() {
        let p = p.as_str();
        let rakam = p.chars().all(|c| c.is_ascii_digit());
        if !rakam && p.starts_with(|c: char| c.is_ascii_digit()) {
            o.push(read_number(p, "nokta"));
        } else if rakam {
            o.push(if p.len() >= 5 { digits_to_words(p) } else { read_digit_group(p) });
        } else if let Some(a) = L.acronyms.get(p) {
            o.push(a.clone());
        } else if is_all_caps(p) || p.chars().count() == 1 {
            o.push(if pronounceable(p) { tr_lower(p) } else { harfler(p) });
        } else {
            o.push(p.to_string());
        }
    }
    o.join(" ")
}

fn pass_alnum(t: &str) -> String {
    sub(&ALNUM, t, |m| ekle(&read_alnum(grup(m, 1).unwrap()), grup(m, 2)))
}

static RANGE: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"(?:(?<![\w.,-])(-)|{NB})({0})\s?-\s?(-?)({0}){1}(?![\w]|[.,]\d)", num(), suf())));
static SKOR_SONRA: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"^\s*(?:{})", L.score_context.join("|"))));

fn pass_ranges(t: &str) -> String {
    let metin = t.to_string();
    sub(&RANGE, t, |m| {
        let (sa, a, sb, b, sf) = (grup(m, 1), grup(m, 2).unwrap(), grup(m, 3).unwrap_or(""), grup(m, 4).unwrap(), grup(m, 5));
        let (s, e) = (m.get(0).unwrap().start(), m.get(0).unwrap().end());
        let once = context_before(&metin, s, 40);
        let sonra = tr_lower(ileri(&metin, e, 30));
        let av = int_value(a.split(',').next().unwrap()).map(|v| if sa.is_some() { -(v as i128) } else { v as i128 });
        let bv = int_value(b.split(',').next().unwrap()).map(|v| if !sb.is_empty() { -(v as i128) } else { v as i128 });
        let first = read_number(&format!("{}{a}", sa.unwrap_or("")), "nokta");
        let son = |v: Option<i128>| -> String {
            match v {
                Some(v) if sf.is_some() && sb.is_empty() => sayi_eki(v.unsigned_abs(), &read_number(b, "nokta"), sf),
                _ => ekle(&read_number(&format!("{sb}{b}"), "nokta"), sf),
            }
        };
        let azalan = matches!((av, bv), (Some(x), Some(y)) if y <= x);
        if azalan || has_word(&once, &L.score_context, 3) || eslesir_bas(&SKOR_SONRA, &sonra, 0).is_some() {
            return format!("{first} {}", son(bv));
        }
        format!("{first} {} {}", ila(&metin, e), son(int_value(b).map(|v| v as i128)))
    })
}

static ADRES_SONRA: LazyLock<Regex> = LazyLock::new(|| rx(r"^\s*(?:nolu|no'lu|numaralı)\b"));
static HARF_KELIME: LazyLock<Regex> = LazyLock::new(|| rx(&format!("[{LET}]+")));

fn address_number(t: &str, s: usize, e: usize) -> bool {
    let k: Vec<String> = HARF_KELIME.find_iter(&context_before(t, s, 24)).flatten().map(|m| m.as_str().to_string()).collect();
    if k.last().is_some_and(|w| L.address_number_context.contains(w)) {
        return true;
    }
    eslesir_bas(&ADRES_SONRA, &tr_lower(ileri(t, e, 16)), 0).is_some()
}

static FRACTION: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"(?<![\w./,])(\d{{1,3}})/(\d{{1,3}}){}(?![\w/]|[.,]\d)", suf())));
static RATIO: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"{NB}(\d+,\d+)/(\d{{1,3}}){}(?![\w/]|[.,]\d)", suf())));

fn pass_fractions(t: &str) -> String {
    let t = sub(&RATIO, t, |m| ekle(&format!("{} bölü {}", read_number(grup(m, 1).unwrap(), "nokta"), number_to_words(sayi(grup(m, 2).unwrap()))), grup(m, 3)));
    let metin = t.clone();
    sub(&FRACTION, &t, |m| {
        let (a, b, sf) = (sayi(grup(m, 1).unwrap()), sayi(grup(m, 2).unwrap()), grup(m, 3));
        let (s, e) = (m.get(0).unwrap().start(), m.get(0).unwrap().end());
        if (a, b) == (24, 7) {
            return ekle("yirmi dört yedi", sf);
        }
        if address_number(&metin, s, e) {
            return ekle(&format!("{} bölü {}", number_to_words(a), number_to_words(b)), sf);
        }
        if b == 0 {
            return m.get(0).unwrap().as_str().into();
        }
        if a >= b && (1..=31).contains(&a) && (1..=12).contains(&b) {
            return ekle(&date_words(a, b as usize, None), sf);
        }
        if a < b {
            return ekle(&format!("{} {}", attach_suffix(&number_to_words(b), "de"), number_to_words(a)), sf);
        }
        ekle(&format!("{} bölü {}", number_to_words(a), number_to_words(b)), sf)
    })
}

const ROMA: &str = "M{0,3}(?:CM|CD|D?C{0,3})(?:XC|XL|L?X{0,3})(?:IX|IV|V?I{0,3})";
static ROMAN: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"(?<![\w.])({ROMA})\.(?=\s+([{LET}][{LET}']*))")));
static ROMAN_TAM: LazyLock<Regex> = LazyLock::new(|| rx(&format!("^{ROMA}$")));
static ROMAN_SUFFIX: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"(?<![\w.])([IVXLC]{{2,}}|[IVX])'([{LOW}]+)(?![\w])")));
static WORD_BEFORE: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"([{LET}]+)[ \t]+$")));
static TITLE_BEFORE: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"(?:(?<![\w])(?i:{})\.?|(?<![\w.])[{UP}]\.)[ \t]+$", L.initial_titles.join("|"))));

fn roman(s: &str) -> u128 {
    let v = |c: char| match c {
        'I' => 1,
        'V' => 5,
        'X' => 10,
        'L' => 50,
        'C' => 100,
        'D' => 500,
        _ => 1000,
    };
    let c: Vec<char> = s.chars().collect();
    let mut t: i64 = 0;
    for i in 0..c.len() {
        if i + 1 < c.len() && v(c[i + 1]) > v(c[i]) {
            t -= v(c[i]);
        } else {
            t += v(c[i]);
        }
    }
    t.max(0) as u128
}

fn ordinal_noun(w: &str) -> bool {
    let w = if is_all_caps(w) { tr_capitalize(&tr_lower(w)) } else { w.to_string() };
    L.ordinal_cap_nouns.contains(w.split('\'').next().unwrap_or("")) || L.ordinal_cap_prefixes.iter().any(|p| w.starts_with(p.as_str()))
}

fn pass_roman(t: &str) -> String {
    let metin = t.to_string();
    let t = sub(&ROMAN_SUFFIX, t, |m| {
        let (tok, sf) = (grup(m, 1).unwrap(), grup(m, 2).unwrap());
        if !tam(&ROMAN_TAM, tok) {
            return m.get(0).unwrap().as_str().into();
        }
        let n = roman(tok);
        if tok.chars().count() == 1 {
            let s = tr_lower(sf);
            let sayi_gibi = harmonize_suffix(&number_to_words(n), &s) == s;
            let harf_gibi = harmonize_suffix(&L.letter_names[tok], &s) == s;
            if harf_gibi && !sayi_gibi {
                return m.get(0).unwrap().as_str().into();
            }
            if harf_gibi == sayi_gibi {
                let st = m.get(0).unwrap().start();
                let pen = geri(&metin, st, 24);
                let onceki = WORD_BEFORE.captures(pen).ok().flatten().and_then(|c| grup(&c, 1).map(str::to_string));
                if !onceki.is_some_and(|w| ordinal_noun(&w)) {
                    return m.get(0).unwrap().as_str().into();
                }
            }
        }
        sayi_eki(n, &number_to_words(n), Some(sf))
    });
    let metin = t.clone();
    sub(&ROMAN, &t, |m| {
        let (tok, nxt) = (grup(m, 1).unwrap_or(""), grup(m, 2).unwrap_or(""));
        if tok.is_empty() || (tok.chars().count() == 1 && !"IVX".contains(tok)) {
            return m.get(0).unwrap().as_str().into();
        }
        if tok.chars().count() == 1 && !(nxt.chars().next().is_some_and(char::is_lowercase) || ordinal_noun(nxt)) {
            let st = m.get(0).unwrap().start();
            if tok == "V" || TITLE_BEFORE.is_match(geri(&metin, st, 24)).unwrap_or(false) {
                return m.get(0).unwrap().as_str().into();
            }
        }
        ordinal_to_words(roman(tok))
    })
}

static ORDINAL_DOT: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"{NB}(\d{{1,4}})\.(?=\s+([{LET}][{LET}']*))")));

fn pass_ordinals(t: &str) -> String {
    sub(&ORDINAL_DOT, t, |m| {
        let nxt = grup(m, 2).unwrap_or("");
        if nxt.chars().next().is_some_and(char::is_lowercase) || ordinal_noun(nxt) {
            ordinal_to_words(sayi(grup(m, 1).unwrap()))
        } else {
            m.get(0).unwrap().as_str().into()
        }
    })
}

// --- sayılar
static NUMBER: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"(?<![\w.,])(-)?((?:{INT_TH}|\d+)(?:,\d+|\.\d+)?)(?:'([{LET}]+)|([{LOW}]+))?(?![\w])")));
static LIST: LazyLock<Regex> = LazyLock::new(|| rx(r"(?<![\w.,])\d+(?:,\d+){2,}(?![\w.,])"));
static VERSION: LazyLock<Regex> = LazyLock::new(|| rx(r"(?<![\w.,])\d+(?:\.\d+){2,}(?![\w]|[.,]\d)"));
static VERSION_X: LazyLock<Regex> = LazyLock::new(|| rx(r"(?<![\w.,])(\d+(?:\.\d+)*)\.[xX](?![\w])"));
static WORD_HYPHEN_NUM: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"(?<=[{LET}]{{2}})-(?=\d)")));
static NUMARA_SONRA: LazyLock<Regex> = LazyLock::new(|| rx(r"^\s*(?:numara|no\b|nolu|kodu|kodlu)"));
static OLCEK_SONRA: LazyLock<Regex> = LazyLock::new(|| rx(r"^\s+(?:bin|milyon|milyar|trilyon)\b"));
static BIRIM_SONRA: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"^\s+(?:{})", *UNIT_WORDS)));
static POSTA_SONRA: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"^[ \t]+([{UP}][{LET}]*)")));
static ADRES_KELIME: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"[{LOW}]+(?:'[{LOW}]+)?")));
static ONCEKI_KELIME: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"([{LET}]+)\.?$")));
const NUMBER_WORDS: [&str; 25] = [
    "sıfır", "bir", "iki", "üç", "dört", "beş", "altı", "yedi", "sekiz", "dokuz", "on", "yirmi", "otuz", "kırk", "elli", "altmış", "yetmiş", "seksen", "doksan",
    "yüz", "bin", "milyon", "milyar", "trilyon", "katrilyon",
];

fn dotted(s: &str) -> String {
    s.split('.').map(|p| if p.len() <= 3 { read_digit_group(p) } else { digits_to_words(p) }).collect::<Vec<_>>().join(" nokta ")
}

fn postcode(t: &str, s: usize, e: usize, n: &str) -> bool {
    let nxt = eslesir_bas(&POSTA_SONRA, t, e);
    if n.len() != 5 || !("01"..="81").contains(&&n[..2]) || nxt.is_none() {
        return false;
    }
    let nxt_w = grup(nxt.as_ref().unwrap(), 1).unwrap().to_string();
    let words: Vec<String> = ADRES_KELIME.find_iter(&context_before(t, s, 100)).flatten().map(|m| m.as_str().to_string()).collect();
    if !words.iter().any(|w| L.address_context.contains(w)) {
        return false;
    }
    let head = t[..s].trim_end_matches([' ', '\t']);
    let prev = ONCEKI_KELIME.captures(head).ok().flatten().and_then(|c| grup(&c, 1).map(tr_lower)).unwrap_or_default();
    let door = head.chars().last().is_some_and(|c| c.is_ascii_digit()) || NUMBER_WORDS.contains(&prev.as_str());
    if is_all_caps(&nxt_w) {
        return door;
    }
    door || head.is_empty() || head.chars().last().is_some_and(|c| ",/:;\n".contains(c)) || L.address_context.contains(&prev)
}

fn pass_numbers(t: &str) -> String {
    let t = sub(&LIST, t, |m| m.get(0).unwrap().as_str().split(',').map(|x| number_to_words(sayi(x))).collect::<Vec<_>>().join(", "));
    let t = sub(&VERSION_X, &t, |m| format!("{} nokta iks", dotted(grup(m, 1).unwrap())));
    let t = sub(&VERSION, &t, |m| {
        let g = m.get(0).unwrap().as_str();
        if tam(&INT_TH_TAM, g) { g.to_string() } else { dotted(g) }
    });
    let t = WORD_HYPHEN_NUM.replace_all(&t, " ").into_owned();
    let metin = t.clone();
    let soylenis = |m: &Cap, isaret: Option<&str>, n: &str, sf: Option<&str>| -> String {
        let s = m.get(0).unwrap().start();
        let e = m.get(0).unwrap().end();
        let neg = isaret.is_some() && (s == 0 || !metin[..s].chars().last().is_some_and(|c| c.is_alphanumeric()));
        let on = if neg { "eksi " } else if isaret.is_some() { "-" } else { "" };
        let sonra = tr_lower(ileri(&metin, e, 24));
        if n.chars().all(|c| c.is_ascii_digit()) {
            let ctx = context_before(&metin, s, 32);
            let sayim = quantity(&ctx, &sonra);
            let kimlik = n.len() >= 5
                && (n.starts_with('0')
                    || n.len() >= 10
                    || (!sayim && has_cue(&ctx, &L.id_context, 3))
                    || eslesir_bas(&NUMARA_SONRA, &sonra, 0).is_some()
                    || postcode(&metin, s, e, n));
            let kod = n.len() >= 4 && !sayim && has_cue(&ctx, &L.code_context, 2);
            if kimlik || kod {
                let sp = if n.len() <= 8 { digits_to_words(n) } else { read_long_digits(n) };
                return format!("{on}{}", ekle(&sp, sf));
            }
            if n.starts_with('0') && n.len() > 1 {
                return format!("{on}{}", ekle(&read_digit_group(n), sf));
            }
            if n.len() > 21 {
                return format!("{on}{}", ekle(&digits_to_words(n), sf));
            }
            let v = sayi(n);
            return format!("{on}{}", sayi_eki(v, &number_to_words(v), sf));
        }
        let ctx_once = context_before(&metin, s, 24);
        let nokta = if eslesir_bas(&OLCEK_SONRA, &sonra, 0).is_some()
            || eslesir_bas(&BIRIM_SONRA, &sonra, 0).is_some()
            || has_word(&ctx_once, &["yüzde", "oran", "faiz", "puan", "not", "ortalama"].map(String::from), 3)
        {
            "virgül"
        } else {
            "nokta"
        };
        let w = read_number(n, nokta);
        if !n.contains(',') && tam(&INT_TH_TAM, n) {
            if let Some(v) = int_value(n) {
                return format!("{on}{}", sayi_eki(v, &w, sf));
            }
        }
        format!("{on}{}", ekle(&w, sf))
    };
    sub(&NUMBER, &t, |m| {
        let (isaret, n, sf, ciplak) = (grup(m, 1), grup(m, 2).unwrap(), grup(m, 3), grup(m, 4));
        match ciplak {
            Some(c) if !L.bare_number_suffixes.contains(&tr_lower(c)) => {
                format!("{} {}", soylenis(m, isaret, n, None), if c == "x" { "iks" } else { c })
            }
            Some(c) => soylenis(m, isaret, n, Some(c)),
            None => soylenis(m, isaret, n, sf),
        }
    })
}

static HARF_NOKTA_RAKAM: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"([{LET}]+)\.(?=\d)")));
static ARTIK_RAKAM: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"(\d+)(?:'([{LET}]+))?")));

fn pass_leftover_digits(t: &str) -> String {
    let t = degistir(&HARF_NOKTA_RAKAM, t, |m| {
        let w = grup(m, 1).unwrap();
        format!("{w}{}", if NUMBER_WORDS.contains(&tr_lower(w).as_str()) { " nokta " } else { " " })
    });
    degistir(&ARTIK_RAKAM, &t, |m| {
        let (d, sf) = (grup(m, 1).unwrap(), grup(m, 2));
        if d.len() > 21 {
            return format!(" {} ", ekle(&digits_to_words(d), sf));
        }
        let w = read_digit_group(d);
        if d.starts_with('0') { format!(" {} ", ekle(&w, sf)) } else { format!(" {} ", sayi_eki(sayi(d), &w, sf)) }
    })
}

static POWER: LazyLock<Regex> = LazyLock::new(|| rx(r"(?:(?<![\w])([A-Za-z]{1,2}|\d+(?:[.,]\d+)?)|(\)))([²³])(?![\w])"));

fn pass_powers(t: &str) -> String {
    sub(&POWER, t, |m| {
        let taban = grup(m, 1).or(grup(m, 2)).unwrap();
        let us = grup(m, 3).unwrap();
        if let Some(u) = L.units.get(&format!("{taban}{us}")) {
            return u.clone();
        }
        let guc = if us == "²" { "kare" } else { "küp" };
        let mut taban = taban.to_string();
        if taban.chars().all(char::is_alphabetic) {
            if taban.chars().count() > 1 {
                return m.get(0).unwrap().as_str().into();
            }
            taban = L.letter_names.get(&tr_upper(&taban)).cloned().unwrap_or(taban);
        }
        format!("{taban} {guc}")
    })
}

// --- büyük harfli başlıklar, kısaltmalar, harf adları, simgeler
static SENTENCE_SPLIT: LazyLock<Regex> = LazyLock::new(|| rx(r"(?<=[.!?])\s+"));
const ONSETS: [&str; 24] = ["bl", "br", "dr", "fl", "fr", "gl", "gr", "kl", "kr", "pl", "pr", "sf", "sk", "sl", "sm", "sn", "sp", "st", "şp", "şt", "tr", "skr", "spr", "str"];
const CODAS: &[&str] = &[
    "rk", "rt", "rç", "rs", "rş", "rz", "rp", "rf", "rm", "rn", "rd", "rb", "rg", "nk", "nç", "nt", "ns", "nd", "ng", "nz", "lk", "lt", "lç", "lp", "lm", "lf", "ls", "şk", "şt", "st",
    "sk", "kt", "ks", "ft", "pt", "ps", "mp",
];

fn word_shaped(w: &str) -> bool {
    let low = tr_lower(w);
    if !low.chars().any(unlu) {
        return false;
    }
    let lead: String = low.chars().take_while(|c| !unlu(*c)).collect();
    let trail: String = low.chars().rev().take_while(|c| !unlu(*c)).collect::<Vec<_>>().into_iter().rev().collect();
    if (lead.chars().count() > 1 && !ONSETS.contains(&lead.as_str())) || (trail.chars().count() > 1 && !CODAS.contains(&trail.as_str())) {
        return false;
    }
    let ic = low.trim_matches(|c: char| "bcçdfgğhjklmnprsştvyzqwx".contains(c));
    let mut run = 0;
    for c in ic.chars() {
        run = if unlu(c) { 0 } else { run + 1 };
        if run > 4 {
            return false;
        }
    }
    true
}

fn pronounceable_or_word(w: &str) -> bool {
    if L.caps_words.contains(w) || pronounceable(w) || w.chars().count() == 1 {
        return true;
    }
    !L.eval_acronyms.contains(w) && word_shaped(w)
}

const EXPANSION_STEMS: [&str; 43] = [
    "sıfır", "bir", "iki", "üç", "dört", "beş", "altı", "yedi", "sekiz", "dokuz", "on", "yirmi", "otuz", "kırk", "elli", "altmış", "yetmiş", "seksen", "doksan", "yüz", "bin",
    "milyon", "milyar", "trilyon", "katrilyon", "dörd", "nokta", "virgül", "buçuk", "yüzde", "binde", "eksi", "artı", "çarpı", "bölü", "ila", "lira", "kuruş", "dolar", "avro",
    "sent", "sterlin", "derece",
];

fn lower_caps_headlines(t: &str) -> String {
    let mut cumleler = Vec::new();
    let mut son = 0;
    for m in SENTENCE_SPLIT.find_iter(t).flatten() {
        cumleler.push(&t[son..m.start()]);
        son = m.end();
    }
    cumleler.push(&t[son..]);
    cumleler
        .into_iter()
        .map(|sent| {
            let words: Vec<&str> = HARF_KELIME
                .find_iter(sent)
                .flatten()
                .map(|m| m.as_str())
                .filter(|w| !(is_lower(w) && EXPANSION_STEMS.iter().any(|s| w.starts_with(s))))
                .collect();
            let harf: Vec<char> = words.iter().flat_map(|w| w.chars()).collect();
            let buyuk = words.iter().filter(|w| is_all_caps(w)).count();
            if harf.len() >= 12 && harf.iter().filter(|c| c.is_uppercase()).count() as f64 / harf.len() as f64 > 0.8 && buyuk >= 3 {
                degistir(&HARF_KELIME, sent, |m| {
                    let w = m.get(0).unwrap().as_str();
                    if L.acronyms.contains_key(w) || !pronounceable_or_word(w) { w.to_string() } else { tr_lower(w) }
                })
            } else {
                sent.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

static ACRONYM: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"(?<![\w'])([{UP}][{UP}]+|PhD)(?:'([{LET}]+))?(?![\w])")));
static SINGLE_LETTER: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"(?<![\w'.])([A-NPR-ZÇĞİÖŞÜ])(?=\s+[{LOW}])")));
static SINGLE_LETTER_SUFFIX: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"(?<![\w'.])([A-NPR-ZÇĞİÖŞÜ])'([{LOW}]+|[{UP}]+)(?![\w'])")));
static INITIAL: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"(?<![\w'.])([{UP}])\.(?=\s+[{UP}])")));

fn pass_acronyms(t: &str) -> String {
    let t = sub(&ACRONYM, t, |m| {
        let (tok, sf) = (grup(m, 1).unwrap(), grup(m, 2));
        let ek = |s: &str| sf.map_or(s.to_string(), |x| format!("{s}'{}", tr_lower(x)));
        let sp = if let Some(a) = L.acronyms.get(tok) {
            a.clone()
        } else if L.caps_words.contains(tok) || pronounceable(tok) {
            return ek(&tr_lower(tok));
        } else {
            harfler(tok)
        };
        match sf {
            None => sp,
            Some(_) if !sp.contains(' ') => ek(&sp),
            Some(x) => ekle(&sp, Some(x)),
        }
    });
    sub(&SINGLE_LETTER, &t, |m| {
        let h = grup(m, 1).unwrap();
        L.letter_names.get(h).cloned().unwrap_or_else(|| h.to_string())
    })
}

fn pass_letter_names(t: &str) -> String {
    let t = sub(&SINGLE_LETTER_SUFFIX, t, |m| ekle(&L.letter_names[grup(m, 1).unwrap()], grup(m, 2)));
    degistir(&INITIAL, &t, |m| {
        let h = grup(m, 1).unwrap();
        format!("{}.", L.letter_names.get(h).cloned().unwrap_or_else(|| tr_lower(h)))
    })
}

static S_VE: LazyLock<Regex> = LazyLock::new(|| rx(r"\s&\s|(?<=\w)&(?=\w)"));
static S_CARPI: LazyLock<Regex> = LazyLock::new(|| rx(r"(?<=\d)\s?[x×]\s?(?=\d)"));
static S_ESIT: LazyLock<Regex> = LazyLock::new(|| rx(r"\s?=\s?"));
static S_ARTI: LazyLock<Regex> = LazyLock::new(|| rx(r"(?<=\w)\s\+\s(?=\w)|(?<=\d)\+(?=\d)"));
static S_YAKLASIK: LazyLock<Regex> = LazyLock::new(|| rx(r"[~≈]\s?(?=\w)"));
static S_DIYEZ: LazyLock<Regex> = LazyLock::new(|| rx(r"(?<!\w)#(?=\w)"));
static S_ET: LazyLock<Regex> = LazyLock::new(|| rx(r"(?<!\w)@(?=\w)"));
static S_PARAGRAF: LazyLock<Regex> = LazyLock::new(|| rx(r"§\s?"));
static S_NUMARA: LazyLock<Regex> = LazyLock::new(|| rx(r"№\s?"));
static S_ISARET: LazyLock<Regex> = LazyLock::new(|| rx(r"[*_|•→←▪►✓✔]"));
static S_YER: LazyLock<Regex> = LazyLock::new(|| rx(&format!(r"\b([{UP}][{LOW}]+)/(?=[{UP}][{LOW}])")));
static S_EGIK: LazyLock<Regex> = LazyLock::new(|| rx(r"(?<=[\w])/(?=[\w])"));
static S_TEKRAR: LazyLock<Regex> = LazyLock::new(|| rx(r"([!?])\1+"));

fn pass_symbols(t: &str) -> String {
    let t = S_VE.replace_all(t, " ve ").into_owned();
    let t = S_CARPI.replace_all(&t, " çarpı ").into_owned();
    let t = S_ESIT.replace_all(&t, " eşittir ").into_owned();
    let t = S_ARTI.replace_all(&t, " artı ").into_owned();
    let t = t.replace('±', " artı eksi ");
    let t = S_YAKLASIK.replace_all(&t, "yaklaşık ").into_owned();
    let t = S_DIYEZ.replace_all(&t, "").into_owned();
    let t = S_ET.replace_all(&t, "").into_owned();
    let t = S_PARAGRAF.replace_all(&t, "paragraf ").into_owned();
    let t = S_NUMARA.replace_all(&t, "numara ").into_owned();
    let t = S_ISARET.replace_all(&t, " ").into_owned();
    let t = S_YER.replace_all(&t, "$1, ").into_owned();
    let t = S_EGIK.replace_all(&t, " ").into_owned();
    S_TEKRAR.replace_all(&t, "$1").into_owned()
}

static C_ONCE_BOSLUK: LazyLock<Regex> = LazyLock::new(|| rx(r"\s+([,.;:!?)])"));
static C_PARANTEZ: LazyLock<Regex> = LazyLock::new(|| rx(r"([(])\s+"));
static C_VIRGUL: LazyLock<Regex> = LazyLock::new(|| rx(r",\s*,"));

fn cleanup(t: &str) -> String {
    let t = C_ONCE_BOSLUK.replace_all(t, "$1").into_owned();
    let t = C_PARANTEZ.replace_all(&t, "$1").into_owned();
    let t = C_VIRGUL.replace_all(&t, ",").into_owned();
    P_BOSLUK.replace_all(&t, " ").trim().to_string()
}

/// Yazılı Türkçe → okunuş (ses modeline giden metin). Büyük harf, noktalama ve özel adlardaki kesme korunur.
pub fn normalize(text: &str) -> String {
    let gecisler: [fn(&str) -> String; 27] = [
        pass_names, pass_web, pass_abbreviations, pass_products, pass_iban, pass_phones, pass_plates, pass_dates, pass_times, pass_money, pass_percent, pass_degrees,
        pass_units, pass_powers, pass_alnum, pass_ranges, pass_fractions, pass_roman, pass_ordinals, pass_numbers, pass_leftover_digits, lower_caps_headlines,
        pass_acronyms, pass_letter_names, pass_symbols, pass_clitics, cleanup,
    ];
    let mut t = preclean(text);
    for g in gecisler {
        t = g(&t);
    }
    t
}

/// Ses modeline giden okunuş: normalize, baştaki "(" / "[" atılır (model talimat sanmasın); boşsa metnin kendisi
pub fn okunus(text: &str) -> String {
    let n = normalize(text);
    let n = n.trim();
    let n = if n.is_empty() { text } else { n };
    let s = n.trim_start_matches(['(', '[']).trim();
    if s.is_empty() { text.to_string() } else { s.to_string() }
}

/// trnorm'un marka / ürün sözlüğü (iPhone → ayfon …): yazılış → okunuş
pub fn marka(yazilis: &str) -> Option<String> {
    NAMES.0.get(yazilis).cloned()
}
