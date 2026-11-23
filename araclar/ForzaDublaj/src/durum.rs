//! İlerleme: aşamalar, her aşamada kaç işten kaçı bitti, geçen ve kalan süre. Hem bu pencereye (konsol) yazılır,
//! hem de tarayıcıda açılan `durum.html`'e (2 sn'de bir kendini yeniler). Kalan süre: süren aşamada şimdiye kadarki
//! hızdan, sıradaki aşamalarda tahminden (biten aşamaların tahmine göre ne kadar hızlı / yavaş gittiğiyle düzeltilir).

use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};
use std::time::Instant;

struct Asama {
    ad: String,
    /// tahmini süre (sn)
    tahmin: f64,
    toplam: usize,
    bitti: usize,
    bas: Option<Instant>,
    /// bittiyse gerçek süre
    sure: Option<f64>,
    aciklama: String,
}

struct Durum {
    baslik: String,
    asamalar: Vec<Asama>,
    simdiki: Option<usize>,
    bas: Instant,
    html: Option<PathBuf>,
    gunluk_dosya: Option<PathBuf>,
    gunluk: Vec<String>,
    son_yazma: Option<Instant>,
    son_konsol: Option<Instant>,
    hata: Option<String>,
    sonuc: Option<(String, String)>,
}

static D: LazyLock<Mutex<Durum>> = LazyLock::new(|| {
    Mutex::new(Durum {
        baslik: String::new(),
        asamalar: Vec::new(),
        simdiki: None,
        bas: Instant::now(),
        html: None,
        gunluk_dosya: None,
        gunluk: Vec::new(),
        son_yazma: None,
        son_konsol: None,
        hata: None,
        sonuc: None,
    })
});

pub fn sure_yaz(sn: f64) -> String {
    let s = sn.max(0.0).round() as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

/// Aşamalar (ad, tahmini sn) ile başlar
pub fn kur(baslik: &str, html: PathBuf, gunluk: PathBuf, asamalar: &[(String, f64)]) {
    let mut d = D.lock().unwrap();
    d.baslik = baslik.to_string();
    d.asamalar = asamalar
        .iter()
        .map(|(a, t)| Asama { ad: a.clone(), tahmin: *t, toplam: 0, bitti: 0, bas: None, sure: None, aciklama: String::new() })
        .collect();
    d.bas = Instant::now();
    d.html = Some(html);
    let _ = std::fs::write(&gunluk, "");
    d.gunluk_dosya = Some(gunluk);
    yaz(&mut d, true);
}

fn bul(d: &Durum, ad: &str) -> Option<usize> {
    d.asamalar.iter().position(|a| a.ad == ad)
}

/// Aşama başlar (`toplam`: kaç iş)
pub fn basla(ad: &str, toplam: usize) {
    let mut d = D.lock().unwrap();
    if let Some(i) = bul(&d, ad) {
        let a = &mut d.asamalar[i];
        a.toplam = toplam.max(1);
        a.bitti = 0;
        a.bas = Some(Instant::now());
        a.sure = None;
        d.simdiki = Some(i);
    }
    drop(d);
    gunluk(&format!("▶ {ad}"));
}

pub fn ilerle(ad: &str, bitti: usize, aciklama: &str) {
    let mut d = D.lock().unwrap();
    if let Some(i) = bul(&d, ad) {
        let a = &mut d.asamalar[i];
        a.bitti = bitti.min(a.toplam);
        a.aciklama = aciklama.to_string();
    }
    yaz(&mut d, false);
}

/// Toplam iş sayısı sonradan belli olduysa
pub fn toplam(ad: &str, n: usize) {
    let mut d = D.lock().unwrap();
    if let Some(i) = bul(&d, ad) {
        d.asamalar[i].toplam = n.max(1);
    }
}

pub fn bitir(ad: &str) {
    let mut d = D.lock().unwrap();
    let mut sure = 0.0;
    if let Some(i) = bul(&d, ad) {
        let a = &mut d.asamalar[i];
        a.bitti = a.toplam;
        sure = a.bas.map_or(0.0, |b| b.elapsed().as_secs_f64());
        a.sure = Some(sure);
        a.aciklama.clear();
    }
    yaz(&mut d, true);
    drop(d);
    gunluk(&format!("✔ {ad} ({})", sure_yaz(sure)));
}

/// Aşama bu çalıştırmada gerekmedi (önbellekte hazırdı)
pub fn atla(ad: &str, neden: &str) {
    let mut d = D.lock().unwrap();
    if let Some(i) = bul(&d, ad) {
        let a = &mut d.asamalar[i];
        a.sure = Some(0.0);
        a.tahmin = 0.0;
        a.toplam = 1;
        a.bitti = 1;
        a.aciklama = neden.to_string();
    }
    yaz(&mut d, true);
    drop(d);
    gunluk(&format!("✔ {ad}: {neden}"));
}

pub fn gunluk(s: &str) {
    let mut d = D.lock().unwrap();
    let gecen = sure_yaz(d.bas.elapsed().as_secs_f64());
    let satir = format!("[{gecen}] {s}");
    println!("{satir}");
    if let Some(p) = &d.gunluk_dosya {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().append(true).create(true).open(p) {
            let _ = writeln!(f, "{satir}");
        }
    }
    d.gunluk.push(satir);
    let n = d.gunluk.len();
    if n > 14 {
        d.gunluk.drain(..n - 14);
    }
    yaz(&mut d, true);
}

pub fn hata(e: &str) {
    let mut d = D.lock().unwrap();
    d.hata = Some(e.to_string());
    yaz(&mut d, true);
}

/// İş bitti: sonuç sayfasına bağlantı (ad, dosya adresi)
pub fn tamam(ad: &str, adres: &str) {
    let mut d = D.lock().unwrap();
    d.simdiki = None;
    d.sonuc = Some((ad.to_string(), adres.to_string()));
    yaz(&mut d, true);
}

/// (geçen, kalan) toplam süre
fn hesap(d: &Durum) -> (f64, f64) {
    let gecen = d.bas.elapsed().as_secs_f64();
    // biten aşamaların gerçek / tahmin oranı sıradaki aşamaların tahminini düzeltir
    let (mut g, mut t) = (0.0, 0.0);
    for a in &d.asamalar {
        if let Some(s) = a.sure
            && a.tahmin > 0.0
        {
            g += s;
            t += a.tahmin;
        }
    }
    let kat = if t > 30.0 { (g / t).clamp(0.4, 3.0) } else { 1.0 };
    let mut kalan = 0.0;
    for (i, a) in d.asamalar.iter().enumerate() {
        if a.sure.is_some() {
            continue;
        }
        if Some(i) == d.simdiki {
            kalan += asama_kalan(a, kat);
        } else {
            kalan += a.tahmin * kat;
        }
    }
    (gecen, kalan)
}

fn asama_kalan(a: &Asama, kat: f64) -> f64 {
    let gecen = a.bas.map_or(0.0, |b| b.elapsed().as_secs_f64());
    if a.bitti > 0 && a.toplam > 0 {
        gecen / a.bitti as f64 * (a.toplam - a.bitti) as f64
    } else {
        (a.tahmin * kat - gecen).max(a.tahmin * kat * 0.1)
    }
}

fn yaz(d: &mut Durum, zorla: bool) {
    let simdi = Instant::now();
    let (gecen, kalan) = hesap(d);
    // konsol: en çok saniyede bir satır
    if zorla || d.son_konsol.is_none_or(|t| simdi.duration_since(t).as_secs_f64() >= 1.0) {
        if let Some(i) = d.simdiki {
            let a = &d.asamalar[i];
            let yuzde = if a.toplam > 0 { a.bitti as f64 * 100.0 / a.toplam as f64 } else { 0.0 };
            println!(
                "    {} {}/{} (%{:.0}) {} · geçen {} · kalan ~{}",
                a.ad,
                a.bitti,
                a.toplam,
                yuzde,
                a.aciklama,
                sure_yaz(gecen),
                sure_yaz(kalan)
            );
        }
        d.son_konsol = Some(simdi);
    }
    if !zorla && d.son_yazma.is_some_and(|t| simdi.duration_since(t).as_secs_f64() < 0.7) {
        return;
    }
    d.son_yazma = Some(simdi);
    let Some(yol) = d.html.clone() else { return };
    let toplam_tahmin = gecen + kalan;
    let genel = if toplam_tahmin > 0.0 { (gecen / toplam_tahmin * 100.0).min(100.0) } else { 0.0 };
    let genel = if d.sonuc.is_some() { 100.0 } else { genel };
    let mut h = String::new();
    let yenile = if d.sonuc.is_some() || d.hata.is_some() { "" } else { "<meta http-equiv=\"refresh\" content=\"2\">" };
    let _ = write!(
        h,
        "<!DOCTYPE html><html lang=\"tr\"><head><meta charset=\"utf-8\">{yenile}<title>ForzaDublaj · {}</title><style>\
body{{font-family:Segoe UI,system-ui,sans-serif;background:#16181d;color:#e8e8ea;margin:0;padding:28px 36px;max-width:980px}}\
h1{{font-size:22px;margin:0 0 4px}}.alt{{color:#9aa0aa;margin-bottom:22px}}\
.kutu{{background:#1f2229;border-radius:12px;padding:16px 18px;margin-bottom:14px}}\
.cubuk{{height:10px;background:#2c3039;border-radius:5px;overflow:hidden;margin-top:8px}}\
.dolu{{height:100%;background:#4f8cff;border-radius:5px}}.yesil{{background:#3fbf7f}}\
.satir{{display:flex;justify-content:space-between;align-items:baseline;gap:12px}}\
.ad{{font-weight:600}}.sag{{color:#9aa0aa;font-variant-numeric:tabular-nums;white-space:nowrap}}\
.bekliyor .ad{{color:#7d828c}}.buyuk{{font-size:30px;font-weight:700;font-variant-numeric:tabular-nums}}\
pre{{background:#111317;border-radius:10px;padding:12px;color:#b9bec8;font-size:12.5px;white-space:pre-wrap;margin:0}}\
.hata{{background:#4a1d22;color:#ffd5d9}}a{{color:#7fb0ff}}\
</style></head><body><h1>ForzaDublaj · {}</h1><div class=\"alt\">Türkçe dublaj ilerlemesi (sayfa kendini yeniler)</div>",
        esc(&d.baslik),
        esc(&d.baslik)
    );
    let _ = write!(
        h,
        "<div class=\"kutu\"><div class=\"satir\"><div><div class=\"sag\">Geçen</div><div class=\"buyuk\">{}</div></div>\
<div><div class=\"sag\">Kalan (tahmini)</div><div class=\"buyuk\">{}</div></div><div><div class=\"sag\">Genel</div><div class=\"buyuk\">%{:.0}</div></div></div>\
<div class=\"cubuk\"><div class=\"dolu{}\" style=\"width:{:.1}%\"></div></div></div>",
        sure_yaz(gecen),
        if d.sonuc.is_some() { "bitti".to_string() } else { sure_yaz(kalan) },
        genel,
        if d.sonuc.is_some() { " yesil" } else { "" },
        genel
    );
    if let Some(e) = &d.hata {
        let _ = write!(h, "<div class=\"kutu hata\"><b>Hata:</b> {}</div>", esc(e));
    }
    if let Some((ad, adres)) = &d.sonuc {
        let _ = write!(h, "<div class=\"kutu\"><b>Bitti.</b> <a href=\"{}\">{}</a></div>", esc(adres), esc(ad));
    }
    let kat = 1.0;
    for (i, a) in d.asamalar.iter().enumerate() {
        let (yuzde, sag, sinif) = if let Some(s) = a.sure {
            (100.0, format!("bitti · {}", sure_yaz(s)), "")
        } else if Some(i) == d.simdiki {
            let y = if a.toplam > 0 { a.bitti as f64 * 100.0 / a.toplam as f64 } else { 0.0 };
            (y, format!("{}/{} · kalan ~{}", a.bitti, a.toplam, sure_yaz(asama_kalan(a, kat))), "")
        } else {
            (0.0, format!("bekliyor · tahmin ~{}", sure_yaz(a.tahmin)), " bekliyor")
        };
        let _ = write!(
            h,
            "<div class=\"kutu{sinif}\"><div class=\"satir\"><span class=\"ad\">{}</span><span class=\"sag\">{}</span></div>\
<div class=\"sag\">{}</div><div class=\"cubuk\"><div class=\"dolu{}\" style=\"width:{:.1}%\"></div></div></div>",
            esc(&a.ad),
            esc(&sag),
            esc(&a.aciklama),
            if a.sure.is_some() { " yesil" } else { "" },
            yuzde
        );
    }
    let _ = write!(h, "<div class=\"kutu\"><div class=\"ad\" style=\"margin-bottom:8px\">Son olaylar</div><pre>{}</pre></div></body></html>", esc(&d.gunluk.join("\n")));
    let gecici = yol.with_extension("tmp");
    if std::fs::write(&gecici, h).is_ok() {
        let _ = std::fs::rename(&gecici, &yol);
    }
}

pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}
