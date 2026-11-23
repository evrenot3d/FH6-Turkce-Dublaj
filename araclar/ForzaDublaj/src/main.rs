//! ForzaDublaj — Forza Horizon 6 karakter seslerini Türkçeye çevirir (Evrenot'un modelleriyle).
//!
//! Akış (bir karakter, ör. Mei):
//!   1. Ses bankası (VO_Gameplay_Mei_EN.assets.bank) → içindeki FSB5 paketi → vgmstream ile replik replik WAV
//!   2. Whisper large-v3-turbo ile İngilizce metin (cümle zamanlarıyla), emotion2vec+ ile duygu
//!   3. Hy-MT2 7B ile Türkçe (hece hedefli: cümle İngilizce süresine sığsın), CMU + g2p ile ad okunuşları
//!   4. ZONOS3-Mini ses izi: karakterin sesi + ona en yakın Türkçe banka sesi (Türkçe aksan oranı kadar)
//!   5. ZONOS3-Mini ile Türkçe seslendirme, cümle cümle, aynı ses izi ve tohumla
//!   6. Cümleler repliğin içinde İngilizce yerlerine (gerekirse en çok 1,35 kat hızlanarak), stüdyo miksi
//!
//! Kipler:
//!   sabit    — (varsayılan) her replik ayrı işlenir; hepsinde karakterin SABİT ses izi ve SABİT tohumu. Ses izi
//!              bir kez hesaplanıp calisma/<karakter>/ses_izi.json'a yazılır, sonraki her çalıştırmada aynısı kullanılır.
//!   birlesik — replikler arka arkaya tek uzun ses gibi işlenir, sonra geri bölünür (deneme)
//!   parca    — her replik ayrı, ses izi o repliğin kendi sesinden, ayrı tohum (deneme)
//!
//! Kullanım: ForzaDublaj.exe --karakter Mei --adet 10            (en uzun 10 replik)
//!           ForzaDublaj.exe --karakter Mei --dakika 10          (bankadaki sırayla ilk 10 dk; 0 = hepsi)
//!           ForzaDublaj.exe --karakter SatNav --ornek 30        (bankanın baştan sona eşit aralıklı 30 repliği)
//!           --model voxcpm2 --adim 30 (Türkçe ses VoxCPM2; varsayılan ZONOS3)
//!           --model drama3 (v2, yalnız bulut sunucusunda, FD_V2=1: Hy-MT2 30B çeviri, Qwen3-Omni duygu yönleri,
//!                            Fish Audio Drama 3 seslendirme; büyük modeller linux/v2/sunucu.sh ile açılıp kapanır)
//!           --sadece-ceviri (çeviriden sonra dur)
//!           --kip sabit | birlesik | parca | hepsi

mod ceviri;
mod cocuk;
mod cumle;
mod dis;
mod drama;
mod duygu;
mod durum;
mod fsb;
mod ilerleme;
mod konusmaci;
mod miks;
mod onnx;
mod paket;
mod ses;
mod stft;
mod studyo;
mod surec;
mod telaffuz;
mod trnorm;
mod vad;
mod vox;
mod whisper;
mod yollar;
mod zonos;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::durum::{gunluk, sure_yaz};
use crate::yollar::Yollar;

/// Oyundaki seslerin örnekleme hızı
const SR: u32 = 48000;
/// birleşik kipte replikler arasındaki sessizlik (sn): Whisper cümleleri burada keser (1 sn'den uzun sessizlik)
const BOSLUK: f64 = 1.5;
/// ses izi için karakterin seçilen repliklerinden alınan toplam süre (sn)
const REFERANS_SN: f64 = 12.0;
const TOHUM: u64 = 1000;
/// Drama 3'e aynı anda gönderilen cümle sayısı (sunucudaki max_num_seqs ile aynı)
const DRAMA_PARALEL: usize = 8;
/// okunan sesin karakterin tersi cinsiyette olma olasılığı bunu aşarsa yeniden okunur (Evrenot'taki gibi)
const TERS: f32 = 0.85;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Kip {
    Sabit,
    Birlesik,
    Parca,
}

impl Kip {
    fn kod(self) -> &'static str {
        match self {
            Kip::Sabit => "sabit",
            Kip::Birlesik => "birlesik",
            Kip::Parca => "parca",
        }
    }
    fn ad(self) -> &'static str {
        match self {
            Kip::Sabit => "sabit ses izi",
            Kip::Birlesik => "birleşik",
            Kip::Parca => "parça parça",
        }
    }
}

struct Ayar {
    karakter: String,
    dakika: f64,
    /// 0 değilse: karakterin en uzun bu kadar repliği
    adet: usize,
    /// 0 değilse: bankanın baştan sona eşit aralıklarla bu kadar repliği (çeşit görmek için)
    ornek: usize,
    /// 0 değilse: duygusu belirgin bu kadar replik (karakterin tamamı önce yazıya dökülmüş olmalı)
    duygulu: usize,
    kipler: Vec<Kip>,
    aksan: f32,
    duygu: bool,
    hece: bool,
    en_cok_hiz: f32,
    studyo: Vec<String>,
    cmu: bool,
    g2p: bool,
    /// Türkçe ses modeli: "zonos3" ya da "voxcpm2"
    model: String,
    /// VoxCPM2 adım sayısı (num_inference_steps)
    adim: u32,
    /// dublaj yerine: Türkçe sesleri oyunun ses bankasına paketle
    paketle: bool,
    /// paketlerken Türkçesi olmayan replikler İngilizce kalmasın, bir deneme cümlesi olsun (oyunda hemen duyulsun)
    deneme: bool,
    /// çeviriden sonra dur (v2: çevirileri önce görmek için)
    sadece_ceviri: bool,
    /// Drama 3: oyunculuk yönleri çıkarılınca dur (toplu çalışmada önce bütün karakterlerin yönleri alınır)
    sadece_yon: bool,
}

/// Bir cümle: hangi replikte (seçimdeki sıra), replik içindeki zamanı, İngilizce metni, duygusu, ses gücü
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Cumle {
    klip: usize,
    start: f64,
    end: f64,
    en: String,
    #[serde(default)]
    duygu: String,
    #[serde(default)]
    duygu_p: f32,
    #[serde(default)]
    rms: f32,
}

struct Klip {
    /// bankadaki sıra numarası (1'den başlar; çıktı dosyasının adı)
    no: usize,
    x48: Vec<f32>,
    sure: f64,
}

/// Karakterin sabit ses izi (ZONOS3, 192 boyut; Türkçe aksan karışımı her çalıştırmada bununla yapılır)
#[derive(Serialize, Deserialize, Clone)]
struct IzKaydi {
    kadin: bool,
    kendi: Vec<f32>,
    turkce: Option<String>,
    referans_sn: f64,
}

#[derive(Serialize, Deserialize, Clone)]
struct BankaSesi {
    ad: String,
    kadin: bool,
    iz: Vec<f32>,
}

fn main() {
    let sonuc = calistir();
    if let Err(e) = &sonuc {
        let m = format!("{e:#}");
        durum::gunluk(&format!("HATA: {m}"));
        durum::hata(&m);
    }
    if !toplu() {
        println!();
        println!("Pencereyi kapatmak için Enter'a basın...");
        let _ = std::io::stdin().read_line(&mut String::new());
    }
    if sonuc.is_err() {
        std::process::exit(1);
    }
}

/// --toplu: arka arkaya çok karakter işlenirken tarayıcı sekmesi açılmaz, sonda Enter beklenmez
fn toplu() -> bool {
    std::env::args().any(|a| a == "--toplu")
}

// ------------------------------------------------------------------ ayarlar
fn ayar_oku(y: &Yollar) -> Ayar {
    let mut a = Ayar {
        karakter: "Mei".into(),
        dakika: 10.0,
        adet: 0,
        ornek: 0,
        duygulu: 0,
        kipler: vec![Kip::Sabit],
        aksan: 0.5,
        duygu: true,
        hece: true,
        en_cok_hiz: 1.35,
        studyo: ["mixm", "mixd", "mixl", "mixs", "mixt", "mixg"].iter().map(|s| s.to_string()).collect(),
        cmu: true,
        g2p: true,
        model: "zonos3".into(),
        adim: 30,
        paketle: false,
        deneme: false,
        sadece_ceviri: false,
        sadece_yon: false,
    };
    // kullanıcının Evrenot ayarları (Türkçe aksan, duygu, hece hedefli çeviri, en çok hızlandırma, ad okunuşu, stüdyo)
    let evrenot = PathBuf::from(yollar::EVRENOT).join("ayarlar.json");
    if let Ok(s) = std::fs::read_to_string(&evrenot)
        && let Ok(v) = serde_json::from_str::<serde_json::Value>(&s)
    {
        if let Some(x) = v["turkce_aksan"].as_f64() {
            a.aksan = x as f32;
        }
        if let Some(x) = v["duygu"].as_bool() {
            a.duygu = x;
        }
        if let Some(x) = v["sure_uydur"].as_bool() {
            a.hece = x;
        }
        if let Some(x) = v["en_cok_hiz"].as_f64() {
            a.en_cok_hiz = x as f32;
        }
        if let Some(x) = v["telaffuz_cmu"].as_bool() {
            a.cmu = x;
        }
        if let Some(x) = v["telaffuz_g2p"].as_bool() {
            a.g2p = x;
        }
        if let Some(x) = v["studyo"].as_array() {
            a.studyo = x.iter().filter_map(|s| s.as_str().map(String::from)).collect();
        }
    }
    // oda sesi kapalı (oyun sesleri temiz stüdyo kaydı); arka plan alçaltma burada iş yapmaz ama zararsız
    a.studyo.retain(|k| k != "mixr");
    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        let deger = args.get(i + 1).cloned().unwrap_or_default();
        match args[i].as_str() {
            "--karakter" => a.karakter = deger,
            "--dakika" => a.dakika = deger.replace(',', ".").parse().unwrap_or(a.dakika),
            "--adet" => a.adet = deger.parse().unwrap_or(0),
            "--ornek" => a.ornek = deger.parse().unwrap_or(0),
            "--duygulu" => a.duygulu = deger.parse().unwrap_or(0),
            "--duygusuz" => {
                a.duygu = false;
                i += 1;
                continue;
            }
            "--model" => {
                let d = deger.to_lowercase();
                a.model = if d.starts_with("vox") {
                    "voxcpm2".into()
                } else if d.starts_with("drama") {
                    "drama3".into()
                } else {
                    "zonos3".into()
                }
            }
            "--adim" => a.adim = deger.parse().unwrap_or(30).max(1),
            "--kip" => {
                a.kipler = match deger.as_str() {
                    "birlesik" => vec![Kip::Birlesik],
                    "parca" => vec![Kip::Parca],
                    "hepsi" => vec![Kip::Sabit, Kip::Birlesik, Kip::Parca],
                    _ => vec![Kip::Sabit],
                }
            }
            "--aksan" => a.aksan = deger.replace(',', ".").parse().unwrap_or(a.aksan),
            "--paketle" => {
                a.paketle = true;
                i += 1;
                continue;
            }
            "--deneme" => {
                a.deneme = true;
                i += 1;
                continue;
            }
            "--sadece-yon" => {
                a.sadece_yon = true;
                i += 1;
                continue;
            }
            "--sadece-ceviri" => {
                a.sadece_ceviri = true;
                i += 1;
                continue;
            }
            _ => {
                i += 1;
                continue;
            }
        }
        i += 2;
    }
    let _ = y;
    a
}

/// Karakterin İngilizce ses bankası: önce "Forza 6 Seslendirme\Orijinal_EN" altındaki klasörler, sonra oyunun kendisi
fn banka_bul(y: &Yollar, karakter: &str) -> Result<PathBuf> {
    let adlar = [format!("VO_Gameplay_{karakter}_EN.assets.bank"), format!("VO_{karakter}_EN.assets.bank")];
    let kok = y.proje.join("Orijinal_EN");
    let mut yerler = vec![kok.clone()];
    if let Ok(it) = std::fs::read_dir(&kok) {
        yerler.extend(it.flatten().map(|e| e.path()).filter(|p| p.is_dir()));
    }
    yerler.push(PathBuf::from(r"D:\STEAM\steamapps\common\ForzaHorizon6\media\Audio\FMODBanks"));
    for d in &yerler {
        for a in &adlar {
            let p = d.join(a);
            if p.is_file() {
                return Ok(p);
            }
        }
    }
    bail!("{karakter} için ses bankası bulunamadı ({} içinde {} aranadı)", kok.display(), adlar[0])
}

/// Evrenot'un ölçtüğü hızlar (1 dk video için saniye): kalan süre tahmini bunlarla başlar
fn hiz_tablosu() -> HashMap<String, f64> {
    let mut h: HashMap<String, f64> = [
        ("metin:whisper", 3.6),
        ("ceviri:hymt2", 14.3),
        ("duygu:emotion2vec", 1.7),
        ("ses:zonos3", 6.7),
        ("studyo", 1.0),
        ("miks", 0.9),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    if let Ok(s) = std::fs::read_to_string(PathBuf::from(yollar::EVRENOT).join("model_hizlari.json"))
        && let Ok(v) = serde_json::from_str::<serde_json::Value>(&s)
        && let Some(o) = v.as_object()
    {
        for (k, x) in o {
            if let (Some(sn), Some(video)) = (x["sn"].as_f64(), x["video"].as_f64())
                && video > 60.0
            {
                h.insert(k.clone(), sn / video * 60.0);
            }
        }
    }
    h
}

/// ZONOS3'ün ölçülen konuşma hızı (hece / sn): hece hedefli çeviri bununla
fn konusma_hizi(y: &Yollar) -> f64 {
    let yol = [PathBuf::from(yollar::EVRENOT).join("konusma_hizi.json"), y.kok.join("konusma_hizi.json")].into_iter().find(|p| p.is_file()).unwrap_or_default();
    std::fs::read_to_string(yol)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| {
            let (h, sn) = (v["zonos3"]["hece"].as_f64()?, v["zonos3"]["sn"].as_f64()?);
            (sn > 20.0).then(|| h / sn)
        })
        .unwrap_or(6.0)
}

fn json_oku<T: for<'a> Deserialize<'a>>(p: &Path) -> Option<T> {
    std::fs::read_to_string(p).ok().and_then(|s| serde_json::from_str(&s).ok())
}

fn json_yaz<T: Serialize>(p: &Path, v: &T) {
    let _ = std::fs::write(p, serde_json::to_string_pretty(v).unwrap_or_default());
}

// ------------------------------------------------------------------ ana akış
fn calistir() -> Result<()> {
    let y = Yollar::bul()?;
    ses::ffmpeg_ayarla(&y.bin);
    let ayar = ayar_oku(&y);
    if ayar.paketle {
        return paketle(&y, &ayar);
    }
    let k = ayar.karakter.clone();
    let calisma = y.calisma.join(&k);
    let en_dizin = calisma.join("en");
    std::fs::create_dir_all(&en_dizin)?;
    let cikti = y.proje.join("Cikti").join(&k);
    std::fs::create_dir_all(&cikti)?;

    println!("ForzaDublaj · {k} · {} · kip: {}",
             if ayar.duygulu > 0 { format!("duygulu {} replik", ayar.duygulu) } else if ayar.ornek > 0 { format!("eşit aralıklı {} replik", ayar.ornek) } else if ayar.adet > 0 { format!("en uzun {} replik", ayar.adet) } else if ayar.dakika > 0.0 { format!("{} dk", ayar.dakika) } else { "hepsi".into() },
             ayar.kipler.iter().map(|x| x.ad()).collect::<Vec<_>>().join(" + "));
    println!("Modeller: {}", y.modeller.display());

    let banka_yolu = banka_bul(&y, &k)?;
    let banka = std::fs::read(&banka_yolu).with_context(|| format!("okunamadı: {}", banka_yolu.display()))?;
    let paket = fsb::ayikla(&banka)?;
    drop(banka);
    let n_hepsi = paket.parcalar.len();
    if n_hepsi == 0 {
        bail!("bankada replik yok");
    }
    // seçim: en uzun N replik (bankadaki sırasıyla) ya da bankadaki sırayla istenen süre dolana kadar
    let mut secim: Vec<usize> = Vec::new();
    if ayar.duygulu > 0 {
        // karakterin tamamının yazıya dökümünden: belirgin duygulu cümlesi olan replikler, duygular sırayla (çeşit olsun)
        let tam = calisma.join(format!("metin_sabit_{n_hepsi}_1_{n_hepsi}.json"));
        let c: Vec<Cumle> = json_oku(&tam).with_context(|| format!("{} yok: önce {k} karakterinin tamamı yazıya dökülmeli", tam.display()))?;
        let mut kume: HashMap<String, Vec<(f32, usize)>> = HashMap::new();
        for z in &c {
            if duygu::belirgin(&z.duygu, z.duygu_p) {
                kume.entry(z.duygu.clone()).or_default().push((z.duygu_p, z.klip));
            }
        }
        let mut turler: Vec<String> = kume.keys().cloned().collect();
        turler.sort();
        for v in kume.values_mut() {
            v.sort_by(|a, b| b.0.total_cmp(&a.0));
        }
        let mut sira = 0usize;
        while secim.len() < ayar.duygulu && turler.iter().any(|t| sira < kume[t].len()) {
            for t in &turler {
                if let Some(&(_, kl)) = kume[t].get(sira)
                    && secim.len() < ayar.duygulu
                    && !secim.contains(&kl)
                {
                    secim.push(kl);
                }
            }
            sira += 1;
        }
        if secim.is_empty() {
            bail!("{k} karakterinde belirgin duygulu cümle bulunamadı");
        }
        secim.sort();
    } else if ayar.ornek > 0 {
        let n = ayar.ornek.min(n_hepsi);
        secim = (0..n).map(|j| if n == 1 { 0 } else { (j as f64 * (n_hepsi - 1) as f64 / (n - 1) as f64).round() as usize }).collect();
        secim.dedup();
    } else if ayar.adet > 0 {
        let mut sira: Vec<usize> = (0..n_hepsi).collect();
        sira.sort_by(|&a, &b| paket.sure(b).total_cmp(&paket.sure(a)));
        secim = sira.into_iter().take(ayar.adet).collect();
        secim.sort();
    } else {
        let mut t = 0.0;
        for i in 0..n_hepsi {
            if ayar.dakika > 0.0 && t >= ayar.dakika * 60.0 {
                break;
            }
            secim.push(i);
            t += paket.sure(i);
        }
    }
    let top: f64 = secim.iter().map(|&i| paket.sure(i)).sum();
    let dk = top / 60.0;

    // aşamalar ve tahmini süreleri (Evrenot'un bu bilgisayarda ölçtüğü hızlarla; konuşma yoğun olduğu için ×1,4)
    let h = hiz_tablosu();
    let hz = |a: &str| h.get(a).copied().unwrap_or(5.0) * dk * 1.4;
    let ad_cikar = "1. İngilizce sesleri çıkarma (vgmstream)".to_string();
    let ad_metin = |x: Kip| format!("2. Yazıya dökme + duygu (Whisper) · {}", x.ad());
    let ad_ceviri = if dis::etkin() { "3. Çeviri (Hy-MT2 30B) + ad okunuşları".to_string() } else { "3. Çeviri (Hy-MT2) + ad okunuşları".to_string() };
    let model_adi = match ayar.model.as_str() {
        "voxcpm2" => format!("VoxCPM2, {} adım", ayar.adim),
        "drama3" => "Drama 3".to_string(),
        _ => "ZONOS3".to_string(),
    };
    let ad_iz = match ayar.model.as_str() {
        "voxcpm2" => "4. VoxCPM2 yükleniyor".to_string(),
        "drama3" if ayar.duygu => "4. Duygu yönleri (Qwen3-Omni) + Drama 3 yükleniyor".to_string(),
        "drama3" => "4. Drama 3 yükleniyor".to_string(),
        _ => "4. Ses izi (ZONOS3)".to_string(),
    };
    let ad_ses = |x: Kip| format!("5. Türkçe seslendirme ({model_adi}) · {}", x.ad());
    let ad_miks = |x: Kip| format!("6. Yerleştirme + stüdyo miksi · {}", x.ad());
    let ad_rapor = "7. Karşılaştırma sayfası".to_string();
    let mut asamalar = vec![(ad_cikar.clone(), 3.0 + 0.05 * secim.len() as f64)];
    for (i, &x) in ayar.kipler.iter().enumerate() {
        asamalar.push((ad_metin(x), hz("metin:whisper") + hz("duygu:emotion2vec") + if i == 0 { 15.0 } else { 0.0 }));
    }
    asamalar.push((ad_ceviri.clone(), hz("ceviri:hymt2") * if ayar.kipler.len() > 1 { 1.15 } else { 1.0 } + 25.0));
    asamalar.push((ad_iz.clone(), if ayar.model == "voxcpm2" || ayar.model == "drama3" { 90.0 + if ayar.model == "drama3" && ayar.duygu { 0.25 * top } else { 0.0 } } else { 30.0 + if ayar.kipler.contains(&Kip::Parca) { 0.4 * secim.len() as f64 } else { 0.0 } }));
    for &x in &ayar.kipler {
        // VoxCPM2: Evrenot'ta 10 adımda 1 dk → ~170 sn ölçülmüştü; adım sayısıyla kabaca orantılı
        let t = match ayar.model.as_str() {
            "voxcpm2" => 170.0 / 60.0 * top * (0.4 + 0.6 * ayar.adim as f64 / 10.0),
            "drama3" => 1.2 * top,
            _ => hz("ses:zonos3") * 1.6,
        };
        asamalar.push((ad_ses(x), t));
    }
    for &x in &ayar.kipler {
        asamalar.push((ad_miks(x), hz("studyo") + hz("miks") + 0.08 * secim.len() as f64));
    }
    asamalar.push((ad_rapor.clone(), 5.0));
    let html = y.kok.join("durum.html");
    durum::kur(&format!("{k} · {:.1} dk · {} replik", dk, secim.len()), html.clone(), calisma.join("gunluk.txt"), &asamalar);
    tarayicida_ac(&html);
    gunluk(&format!(
        "{} bankasında {n_hepsi} replik ({:.1} dk); {} replik seçildi ({:.1} dk). Türkçe aksan {:.0}%, en çok hız {:.2}×, stüdyo: {}",
        banka_yolu.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
        (0..n_hepsi).map(|i| paket.sure(i)).sum::<f64>() / 60.0,
        secim.len(),
        dk,
        ayar.aksan * 100.0,
        ayar.en_cok_hiz,
        ayar.studyo.join(", ")
    ));

    // 1. replikleri WAV'a çıkar
    sesleri_cikar(&y, &paket, &secim, &en_dizin, &ad_cikar)?;
    let klipler: Vec<Klip> = secim
        .iter()
        .map(|&i| -> Result<Klip> {
            let s = ses::wav_oku(&en_dizin.join(format!("{:04}.wav", i + 1)))?;
            let x = ses::yeniden_ornekle(&s.mono(), s.sr, SR);
            let sure = x.len() as f64 / SR as f64;
            Ok(Klip { no: i + 1, x48: x, sure })
        })
        .collect::<Result<_>>()?;

    // işlemcideki küçük modeller
    onnx::hazirla(&y.bin)?;
    let m = &y.modeller;
    let mut vad = vad::Vad::ac(&m.join("silero_vad_v6.onnx"))?;
    // duygu modeli: duygu kapalıysa ve model yoksa (bulut sunucusu) atlanır
    let mut duygu_m = match duygu::Duygu::ac(&m.join("emotion2vec/emotion2vec_plus_base.onnx")) {
        Ok(d) => Some(d),
        Err(e) if !ayar.duygu || ayar.model == "drama3" => {
            // Drama 3'te duygu Qwen3-Omni'den gelir; emotion2vec gerekmez
            gunluk(&format!("duygu modeli açılmadı (gerekmiyor): {e:#}"));
            None
        }
        Err(e) => return Err(e),
    };
    let mut cinsiyet = konusmaci::Cinsiyet::ac(&m.join("cinsiyet/model_quantized.onnx"))?;
    let mut sesizi = konusmaci::SesIzi::ac(&m.join("wespeaker/voxceleb_resnet34_LM.onnx"))?;

    // 2. yazıya dökme (her kip için; önbellekte varsa atlanır)
    let mut cumleler: HashMap<Kip, Vec<Cumle>> = HashMap::new();
    let mut whisper: Option<whisper::Whisper> = None;
    for &x in &ayar.kipler {
        let ad = ad_metin(x);
        let onb = calisma.join(format!("metin_{}_{}_{}_{}.json", x.kod(), secim.len(), secim[0] + 1, secim[secim.len() - 1] + 1));
        if let Some(c) = json_oku::<Vec<Cumle>>(&onb) {
            durum::atla(&ad, &format!("önceki çalıştırmadan hazır ({} cümle)", c.len()));
            cumleler.insert(x, c);
            continue;
        }
        // karakterin tamamı daha önce yazıya döküldüyse seçilen repliklerin cümleleri oradan alınır
        let tam = calisma.join(format!("metin_{}_{n_hepsi}_1_{n_hepsi}.json", x.kod()));
        if x != Kip::Birlesik
            && let Some(t) = json_oku::<Vec<Cumle>>(&tam)
        {
            let c: Vec<Cumle> = t
                .into_iter()
                .filter_map(|mut z| {
                    let j = secim.iter().position(|&s| s == z.klip)?;
                    z.klip = j;
                    Some(z)
                })
                .collect();
            durum::atla(&ad, &format!("karakterin tamamının dökümünden ({} cümle)", c.len()));
            cumleler.insert(x, c);
            continue;
        }
        durum::basla(&ad, 1000);
        if whisper.is_none() {
            durum::ilerle(&ad, 0, "Whisper large-v3-turbo yükleniyor");
            let w = whisper::Whisper::ac(&m.join("whisper-onnx")).context("Whisper açılamadı")?;
            if !w.ekran_karti {
                gunluk("Whisper ekran kartında açılamadı, işlemcide çalışıyor (yavaş)");
            }
            whisper = Some(w);
        }
        let w = whisper.as_mut().unwrap();
        let mut c = match x {
            Kip::Birlesik => metin_birlesik(w, &mut vad, &klipler, &ad)?,
            Kip::Sabit | Kip::Parca => metin_parca(w, &mut vad, &klipler, &ad)?,
        };
        durum::ilerle(&ad, 0, "duygu ve ses gücü");
        ozellikler(&mut c, &klipler, duygu_m.as_mut());
        let bos = (0..klipler.len()).filter(|i| !c.iter().any(|z| z.klip == *i)).count();
        gunluk(&format!("{}: {} cümle; {} replikte konuşma bulunmadı (gülme, nefes vb.: orijinal ses kalır)", x.ad(), c.len(), bos));
        json_yaz(&onb, &c);
        durum::bitir(&ad);
        cumleler.insert(x, c);
    }
    drop(whisper); // ekran kartı çeviri modeline kalsın

    // 3. çeviri (iki kipteki bütün farklı cümleler bir kez)
    // v2'nin çevirileri (Hy-MT2 30B) ayrı dosyada: 7B çevirileriyle karışmasın
    let ceviri_onb = calisma.join(if dis::etkin() { "ceviri_hymt2_30b.json" } else { "ceviri.json" });
    let mut cevriler: HashMap<String, String> = json_oku(&ceviri_onb).unwrap_or_default();
    let mut gerek: Vec<(String, f64)> = Vec::new();
    let mut gorulen = HashSet::new();
    for &x in &ayar.kipler {
        for c in &cumleler[&x] {
            let en = c.en.trim().to_string();
            if !en.is_empty() && !cevriler.contains_key(&en) && gorulen.insert(en.clone()) {
                gerek.push((en, c.end - c.start));
            }
        }
    }
    if gerek.is_empty() {
        durum::atla(&ad_ceviri, "bütün cümleler önceki çalıştırmadan çevrilmiş");
    } else {
        durum::basla(&ad_ceviri, gerek.len());
        durum::ilerle(&ad_ceviri, 0, "Hy-MT2 yükleniyor");
        let mut cev = ceviri::Ceviri::new(&y);
        cev.baslat(250)?;
        let hiz = konusma_hizi(&y);
        gunluk(&format!("{} cümle çevriliyor (hece hedefi: İngilizce süre × {hiz:.2} hece/sn)", gerek.len()));
        let en: Vec<String> = gerek.iter().map(|g| g.0.clone()).collect();
        let sureler: Vec<f64> = gerek.iter().map(|g| g.1).collect();
        let sonuc = Mutex::new(vec![String::new(); en.len()]);
        let sayac = AtomicUsize::new(0);
        let ad = ad_ceviri.clone();
        ceviri::cevir_hepsi(&en, ayar.hece.then_some(sureler.as_slice()), hiz, &|i, t| {
            sonuc.lock().unwrap()[i] = t.clone();
            let n = sayac.fetch_add(1, Ordering::SeqCst) + 1;
            durum::ilerle(&ad, n, &kisalt(&t, 70));
        });
        cev.durdur();
        for (e, t) in en.into_iter().zip(sonuc.into_inner().unwrap()) {
            if !t.trim().is_empty() {
                cevriler.insert(e, t);
            }
        }
        json_yaz(&ceviri_onb, &cevriler);
        durum::bitir(&ad_ceviri);
    }
    if ayar.sadece_ceviri {
        gunluk(&format!("Çeviriler hazır: {} (--sadece-ceviri: burada duruldu)", ceviri_onb.display()));
        return Ok(());
    }
    // sese giden metin: İngilizce adların Türkçe okunuşu (Michael → Maykıl); altyazı / çeviri metni değişmez
    let telaffuz = if ayar.cmu || ayar.g2p { telaffuz::Telaffuz::new(m, ayar.cmu, ayar.g2p).ok() } else { None };
    let say = |en: &str| -> (String, String) {
        let tr = cevriler.get(en.trim()).cloned().unwrap_or_default();
        let s = telaffuz.as_ref().map_or_else(|| tr.clone(), |t| t.donustur(&tr, en).0);
        (tr, s)
    };

    // 4. ses modeli: ZONOS3 (sabit ses izi + Türkçe aksan karışımı) ya da VoxCPM2 (sabit referans sesi)
    let vox_kip = ayar.model == "voxcpm2";
    let drama_kip = ayar.model == "drama3";
    if drama_kip && !dis::etkin() {
        bail!("Drama 3 yalnız bulut sunucusunda (linux/v2, FD_V2=1) çalışır");
    }
    // Drama 3: sabit referans (ses + metni) ve cümle cümle oyunculuk yönleri (Qwen3-Omni)
    let mut drama_ref: Option<(PathBuf, String)> = None;
    let mut yonler: HashMap<String, String> = HashMap::new();
    let gecici = calisma.join("gecici");
    std::fs::create_dir_all(&gecici)?;
    let ref_yol = calisma.join("referans.wav");
    let mut zonos = zonos::Zonos::new(&y);
    let mut vox = vox::Vox::new(&y);
    let (kadin, iz_sabit, iz_parca): (bool, Vec<f32>, Vec<Vec<f32>>) = if drama_kip {
        durum::basla(&ad_iz, 2);
        let tum: Vec<Cumle> = ayar.kipler.iter().flat_map(|x| cumleler[x].iter().cloned()).collect();
        let (ref_wav, ref_metin) = drama_referansi(&klipler, &cumleler[&ayar.kipler[0]], &calisma)?;
        let kadin = match json_oku::<IzKaydi>(&calisma.join("ses_izi.json")) {
            Some(x) => x.kadin,
            None => {
                let r = ses::wav_oku(&ref_wav)?;
                cinsiyet.kadin_uzun(&ses::yeniden_ornekle(&r.mono(), r.sr, 16000)).unwrap_or(0.5) >= 0.5
            }
        };
        if ayar.duygu {
            durum::ilerle(&ad_iz, 0, "duygu yönleri (Qwen3-Omni)");
            yonler = yonleri_al(&y, &calisma, &klipler, &tum)?;
            let belirgin = yonler.values().filter(|v| !notr_yon(v)).count();
            gunluk(&format!("Qwen3-Omni: {} cümlenin {belirgin} tanesinde belirgin oyunculuk yönü", yonler.len()));
        }
        if ayar.sadece_yon {
            gunluk("--sadece-yon: oyunculuk yönleri hazır, burada duruldu");
            return Ok(());
        }
        durum::ilerle(&ad_iz, 1, "Drama 3 yükleniyor (ilk açılış birkaç dakika sürebilir)");
        dis::komut(&y, &["baslat", "drama"])?;
        drama::oku("Merhaba.", &ref_wav, &ref_metin, 1, None).context("Drama 3 açılamadı")?;
        gunluk(&format!(
            "{k}: {} sesi; Drama 3; sabit referans {} (\"{}\"); tohum {TOHUM}",
            if kadin { "kadın" } else { "erkek" },
            ref_wav.display(),
            kisalt(&ref_metin, 80)
        ));
        drama_ref = Some((ref_wav, ref_metin));
        durum::bitir(&ad_iz);
        (kadin, Vec::new(), Vec::new())
    } else if vox_kip {
        durum::basla(&ad_iz, 1);
        if !ref_yol.is_file() {
            ses::wav_yaz_mono(&ref_yol, &referans_sesi(&klipler), 24000)?;
        }
        let kadin = match json_oku::<IzKaydi>(&calisma.join("ses_izi.json")) {
            Some(x) => x.kadin,
            None => {
                let r = ses::wav_oku(&ref_yol)?;
                cinsiyet.kadin_uzun(&ses::yeniden_ornekle(&r.mono(), r.sr, 16000)).unwrap_or(0.5) >= 0.5
            }
        };
        durum::ilerle(&ad_iz, 0, "VoxCPM2 yükleniyor (ilk açılış birkaç dakika sürebilir)");
        vox.baslat()?;
        vox::oku("Merhaba.", &ref_yol, 1, ayar.adim, None).context("VoxCPM2 açılamadı")?;
        gunluk(&format!(
            "{k}: {} sesi; VoxCPM2 {} adım; sabit referans calisma\\{k}\\referans.wav (metinsiz); tohum {TOHUM}",
            if kadin { "kadın" } else { "erkek" },
            ayar.adim
        ));
        durum::bitir(&ad_iz);
        (kadin, Vec::new(), Vec::new())
    } else {
        durum::basla(&ad_iz, 1 + if ayar.kipler.contains(&Kip::Parca) { klipler.len() } else { 0 });
        durum::ilerle(&ad_iz, 0, "ZONOS3-Mini yükleniyor");
        if !zonos.hazir_mi() {
            bail!("ZONOS3-Mini bulunamadı ({} / {})", y.bin.join("zonos3").display(), m.join("zonos3").display());
        }
        zonos.baslat()?;
        zonos::oku("Merhaba.", None, 1, 1.0, None).context("ZONOS3 açılamadı")?;
        // karakterin SABİT ses izi: bir kez (en uzun repliklerinden ~12 sn'lik referanstan) hesaplanır ve saklanır;
        // sonraki bütün çalıştırmalarda aynısı kullanılır → ses hiç kaymaz
        let iz_dosya = calisma.join("ses_izi.json");
        let kayit: IzKaydi = match json_oku::<IzKaydi>(&iz_dosya).filter(|x| x.kendi.len() == 192) {
            Some(x) => {
                gunluk(&format!("sabit ses izi önceki çalıştırmadan ({})", iz_dosya.display()));
                x
            }
            None => {
                let referans = referans_sesi(&klipler);
                let ref16 = ses::yeniden_ornekle(&referans, 24000, 16000);
                let kadin = cinsiyet.kadin_uzun(&ref16).unwrap_or(0.5) >= 0.5;
                let kendi_iz_we = sesizi.iz(&ref16)?;
                let turkce = banka_yukle(m)
                    .into_iter()
                    .filter(|b| b.kadin == kadin)
                    .max_by(|a, b| kos(&a.iz, &kendi_iz_we).total_cmp(&kos(&b.iz, &kendi_iz_we)))
                    .map(|b| b.ad);
                ses::wav_yaz_mono(&ref_yol, &referans, 24000)?;
                let kendi = zonos::ses_izi(&std::fs::read(&ref_yol)?)?;
                let x = IzKaydi { kadin, kendi, turkce, referans_sn: yuvarla(referans.len() as f64 / 24000.0) };
                json_yaz(&iz_dosya, &x);
                x
            }
        };
        let kadin = kayit.kadin;
        let turkce_iz = match &kayit.turkce {
            Some(ad) => Some(zonos::ses_izi(&std::fs::read(m.join("ses_bankasi").join(format!("{ad}.wav")))?)?),
            None => None,
        };
        let karis = |iz: Vec<f32>| match &turkce_iz {
            Some(t) => zonos::karistir(&iz, t, ayar.aksan),
            None => iz,
        };
        let iz_sabit = karis(kayit.kendi.clone());
        gunluk(&format!(
            "{k}: {} sesi; sabit ses izi {:.1} sn'lik referanstan (calisma\\{k}\\referans.wav); Türkçe aksan sesi: {} (%{:.0}); tohum {TOHUM}",
            if kadin { "kadın" } else { "erkek" },
            kayit.referans_sn,
            kayit.turkce.as_deref().unwrap_or("yok"),
            ayar.aksan * 100.0
        ));
        durum::ilerle(&ad_iz, 1, "");
        let mut iz_parca: Vec<Vec<f32>> = Vec::new();
        if ayar.kipler.contains(&Kip::Parca) {
            for (i, kl) in klipler.iter().enumerate() {
                let mut o = ses::yeniden_ornekle(&kl.x48, SR, 24000);
                if o.len() < 36000 {
                    o.resize(36000, 0.0);
                }
                let p = gecici.join("iz.wav");
                ses::wav_yaz_mono(&p, &o, 24000)?;
                let iz = zonos::ses_izi(&std::fs::read(&p)?).map(&karis).unwrap_or_else(|_| iz_sabit.clone());
                iz_parca.push(iz);
                durum::ilerle(&ad_iz, i + 2, &format!("replik {}", kl.no));
            }
        }
        durum::bitir(&ad_iz);
        (kadin, iz_sabit, iz_parca)
    };

    // 5–6. seslendirme, yerleştirme, stüdyo miksi
    let ing_dizin = cikti.join("ingilizce");
    std::fs::create_dir_all(&ing_dizin)?;
    for kl in &klipler {
        ses::wav_yaz_mono(&ing_dizin.join(format!("{:04}.wav", kl.no)), &kl.x48, SR)?;
    }
    // çıkış klasörü: ZONOS3 → "sabit", VoxCPM2 → "sabit_voxcpm2_30" (önceki denemeler silinmez, sayfada yan yana)
    let etiket = |x: Kip| {
        if drama_kip {
            format!("{}_drama3{}", x.kod(), if ayar.duygu { "_duygu" } else { "" })
        } else if vox_kip {
            format!("{}_voxcpm2_{}{}", x.kod(), ayar.adim, if ayar.duygu { "_duygu" } else { "" })
        } else {
            x.kod().to_string()
        }
    };
    let mut dokumler: HashMap<Kip, Vec<serde_json::Value>> = HashMap::new();
    for &x in &ayar.kipler {
        let c = &cumleler[&x];
        let ad = ad_ses(x);
        durum::basla(&ad, c.len());
        let mut sesler: Vec<Option<Vec<f32>>> = vec![None; c.len()];
        let mut sayilar = (0usize, 0f64);
        if let (true, Some((ref_w, ref_t))) = (drama_kip, &drama_ref) {
            // Drama 3: sunucu aynı anda gelen istekleri toplu işler → cümleler DRAMA_PARALEL iş parçacığıyla gönderilir
            let isler: Vec<(String, Option<String>)> = c
                .iter()
                .map(|z| {
                    let (_, s) = say(&z.en);
                    let d = yonler.get(&yon_anahtar(klipler[z.klip].no, z.start)).filter(|v| ayar.duygu && !notr_yon(v)).cloned();
                    (s, d)
                })
                .collect();
            let sonuc = Mutex::new(vec![None; c.len()]);
            let (sira, biten) = (AtomicUsize::new(0), AtomicUsize::new(0));
            let cins_yol = y.modeller.join("cinsiyet/model_quantized.onnx");
            let motor = Motor::Drama(ref_w, ref_t);
            std::thread::scope(|sc| {
                for _ in 0..DRAMA_PARALEL {
                    sc.spawn(|| {
                        let mut cins = match konusmaci::Cinsiyet::ac(&cins_yol) {
                            Ok(x) => x,
                            Err(e) => {
                                gunluk(&format!("cinsiyet modeli açılamadı: {e:#}"));
                                return;
                            }
                        };
                        loop {
                            let i = sira.fetch_add(1, Ordering::SeqCst);
                            if i >= isler.len() {
                                break;
                            }
                            let (s, d) = &isler[i];
                            let r = seslendir(s, motor, TOHUM, kadin, d.as_deref().map(|v| (v, 1.0f32)), &mut cins).unwrap_or_else(|e| {
                                gunluk(&format!("cümle {}: {e:#}", i + 1));
                                None
                            });
                            sonuc.lock().unwrap()[i] = r;
                            let n = biten.fetch_add(1, Ordering::SeqCst) + 1;
                            durum::ilerle(&ad, n, &format!("replik {} · {}", klipler[c[i].klip].no, kisalt(s, 60)));
                        }
                    });
                }
            });
            sesler = sonuc.into_inner().unwrap();
            for a in sesler.iter().flatten() {
                sayilar.0 += 1;
                sayilar.1 += a.len() as f64 / miks::SR as f64;
            }
        }
        for (i, z) in c.iter().enumerate() {
            if drama_kip {
                break;
            }
            let (_, s) = say(&z.en);
            let (iz, tohum) = match x {
                Kip::Parca if !vox_kip => (&iz_parca[z.klip], TOHUM + 7919 * z.klip as u64),
                _ => (&iz_sabit, TOHUM),
            };
            let d = if drama_kip {
                yonler.get(&yon_anahtar(klipler[z.klip].no, z.start)).filter(|v| ayar.duygu && !notr_yon(v)).map(|v| (v.as_str(), 1.0f32))
            } else {
                (ayar.duygu && duygu::belirgin(&z.duygu, z.duygu_p)).then_some((z.duygu.as_str(), 0.8f32))
            };
            let motor = match &drama_ref {
                Some((r, m)) => Motor::Drama(r, m),
                None if vox_kip => Motor::Vox(&ref_yol, ayar.adim),
                None => Motor::Zonos(iz),
            };
            sesler[i] = seslendir(&s, motor, tohum, kadin, d, &mut cinsiyet).unwrap_or_else(|e| {
                gunluk(&format!("cümle {}: {e:#}", i + 1));
                None
            });
            if let Some(a) = &sesler[i] {
                sayilar.0 += 1;
                sayilar.1 += a.len() as f64 / miks::SR as f64;
            }
            durum::ilerle(&ad, i + 1, &format!("replik {} · {}", klipler[z.klip].no, kisalt(&s, 60)));
        }
        gunluk(&format!("{}: {}/{} cümle seslendirildi, {:.1} dk Türkçe ses", x.ad(), sayilar.0, c.len(), sayilar.1 / 60.0));
        durum::bitir(&ad);

        let ad = ad_miks(x);
        durum::basla(&ad, klipler.len());
        let dizin = cikti.join(etiket(x));
        std::fs::create_dir_all(&dizin)?;
        let d = yerlestir_ve_isle(x, &klipler, c, &sesler, &ayar, &gecici, &ad)?;
        let mut dokum = Vec::new();
        for (k_i, kl) in klipler.iter().enumerate() {
            let (cikis, orijinal) = match &d.sesler[k_i] {
                Some(a) => (a.clone(), false),
                None => (kl.x48.clone(), true),
            };
            ses::wav_yaz_mono(&dizin.join(format!("{:04}.wav", kl.no)), &cikis, SR)?;
            let bunlar: Vec<usize> = (0..c.len()).filter(|&i| c[i].klip == k_i).collect();
            dokum.push(serde_json::json!({
                "no": kl.no,
                "sure_en": yuvarla(kl.sure),
                "sure_tr": yuvarla(cikis.len() as f64 / SR as f64),
                "orijinal_kaldi": orijinal,
                "cumleler": bunlar.iter().map(|&i| {
                    let (tr, s) = say(&c[i].en);
                    serde_json::json!({"en": c[i].en, "tr": tr, "say": s, "duygu": c[i].duygu, "duygu_p": yuvarla(c[i].duygu_p as f64),
                                       "yon": yonler.get(&yon_anahtar(kl.no, c[i].start)),
                                       "zaman": [yuvarla(c[i].start), yuvarla(c[i].end)],
                                       "yer": d.yerler[i].map(|(a, b, f)| serde_json::json!([yuvarla(a), yuvarla(b), yuvarla(f as f64)]))})
                }).collect::<Vec<_>>(),
            }));
        }
        json_yaz(&cikti.join(format!("dokum_{}.json", etiket(x))), &dokum);
        onizleme(&cikti.join(format!("{k}_{}_onizleme.wav", etiket(x))), &dizin, &klipler)?;
        durum::bitir(&ad);
        dokumler.insert(x, dokum);
    }
    zonos.durdur();
    vox.durdur();
    if drama_kip {
        let _ = dis::komut(&y, &["durdur", "drama"]);
    }
    onizleme(&cikti.join(format!("{k}_ingilizce_onizleme.wav")), &ing_dizin, &klipler)?;

    // 7. karşılaştırma sayfası
    durum::basla(&ad_rapor, 1);
    let sayfa = cikti.join("karsilastirma.html");
    rapor(&sayfa, &k, &ayar, &klipler, &dokumler, &cikti)?;
    durum::bitir(&ad_rapor);
    let adres = format!("file:///{}", sayfa.to_string_lossy().replace('\\', "/").replace(' ', "%20"));
    durum::tamam("Karşılaştırma sayfasını aç", &adres);
    gunluk(&format!("Bitti. Sonuçlar: {}", cikti.display()));
    tarayicida_ac(&sayfa);
    Ok(())
}

fn yuvarla(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

fn kisalt(s: &str, n: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= n { s.to_string() } else { s.chars().take(n).collect::<String>() + "…" }
}

fn kos(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn tarayicida_ac(p: &Path) {
    if toplu() {
        return;
    }
    let mut c = std::process::Command::new("cmd");
    c.args(["/C", "start", ""]).arg(p);
    ses::pencere_acma(&mut c);
    let _ = c.spawn();
}

fn banka_yukle(modeller: &Path) -> Vec<BankaSesi> {
    let d = modeller.join("ses_bankasi");
    let mut v: Vec<BankaSesi> = Vec::new();
    if let Ok(it) = std::fs::read_dir(&d) {
        for e in it.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "json")
                && p.with_extension("wav").is_file()
                && let Some(b) = json_oku::<BankaSesi>(&p)
            {
                v.push(b);
            }
        }
    }
    v.sort_by(|a, b| a.ad.cmp(&b.ad));
    v
}

// ------------------------------------------------------------------ 1. çıkarma
fn sesleri_cikar(y: &Yollar, paket: &fsb::Paket, secim: &[usize], hedef: &Path, ad: &str) -> Result<()> {
    let eksik: Vec<usize> = secim.iter().copied().filter(|&i| !hedef.join(format!("{:04}.wav", i + 1)).is_file()).collect();
    if eksik.is_empty() {
        durum::atla(ad, "önceki çalıştırmadan hazır");
        return Ok(());
    }
    let vgm = y.vgmstream();
    if !vgm.is_file() {
        bail!("vgmstream bulunamadı: {} (kurulum.bat'ı çalıştırın)", vgm.display());
    }
    durum::basla(ad, eksik.len());
    let fsb = hedef.join("_paket.fsb");
    std::fs::write(&fsb, &paket.bayt)?;
    // önce hepsi tek seferde (?s = parça numarası), eksik kalan olursa tek tek
    let sablon = hedef.join("_v_?s.wav");
    let mut c = std::process::Command::new(&vgm);
    c.args(["-i", "-s", "1", "-S", "0", "-o"]).arg(&sablon).arg(&fsb);
    ses::pencere_acma(&mut c);
    durum::ilerle(ad, 0, &format!("{} replik çözülüyor", eksik.len()));
    let o = c.output();
    if let Ok(o) = &o
        && !o.status.success()
    {
        gunluk(&format!("vgmstream (toplu): {}", String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or("")));
    }
    let mut bitti = 0;
    for &i in &eksik {
        let hedef_dosya = hedef.join(format!("{:04}.wav", i + 1));
        let toplu = hedef.join(format!("_v_{}.wav", i + 1));
        if toplu.is_file() {
            std::fs::rename(&toplu, &hedef_dosya)?;
        } else {
            let mut c = std::process::Command::new(&vgm);
            c.args(["-i", "-s", &(i + 1).to_string(), "-o"]).arg(&hedef_dosya).arg(&fsb);
            ses::pencere_acma(&mut c);
            let o = c.output()?;
            if !o.status.success() || !hedef_dosya.is_file() {
                bail!("vgmstream replik {} çıkaramadı: {}", i + 1, String::from_utf8_lossy(&o.stdout).lines().chain(String::from_utf8_lossy(&o.stderr).lines()).last().unwrap_or(""));
            }
        }
        bitti += 1;
        durum::ilerle(ad, bitti, &format!("replik {}", i + 1));
    }
    // seçim dışında toplu çıkarılanlar da adlandırılır (sonraki çalıştırmada hazır olsun)
    if let Ok(it) = std::fs::read_dir(hedef) {
        for e in it.flatten() {
            let ad_ = e.file_name().to_string_lossy().into_owned();
            if let Some(n) = ad_.strip_prefix("_v_").and_then(|s| s.strip_suffix(".wav")).and_then(|s| s.parse::<usize>().ok()) {
                let _ = std::fs::rename(e.path(), hedef.join(format!("{n:04}.wav")));
            }
        }
    }
    let _ = std::fs::remove_file(&fsb);
    durum::bitir(ad);
    Ok(())
}

// ------------------------------------------------------------------ 2. yazıya dökme
fn cumle_yap(kel: &[whisper::Kelime], klip: usize) -> Vec<Cumle> {
    cumle::cumleler(kel, &HashSet::new())
        .into_iter()
        .filter(|c| !c.en.trim().is_empty())
        .map(|c| Cumle { klip, start: c.start, end: c.end.max(c.start + 0.2), en: c.en, duygu: String::new(), duygu_p: 0.0, rms: 0.0 })
        .collect()
}

/// Bütün replikler arka arkaya (araları 1,5 sn sessiz) tek ses; kelimeler sonra repliklerine dağıtılır
fn metin_birlesik(w: &mut whisper::Whisper, vad: &mut vad::Vad, klipler: &[Klip], ad: &str) -> Result<Vec<Cumle>> {
    let mut x16 = Vec::new();
    let mut ofs = Vec::new();
    for k in klipler {
        ofs.push(x16.len() as f64 / 16000.0);
        x16.extend(ses::yeniden_ornekle(&k.x48, SR, 16000));
        x16.extend(std::iter::repeat_n(0.0f32, (BOSLUK * 16000.0) as usize));
    }
    durum::ilerle(ad, 0, &format!("{} sn'lik birleşik ses", sure_yaz(x16.len() as f64 / 16000.0)));
    let kel = w.yaziya_dok(&x16, vad, |p| durum::ilerle(ad, (p * 1000.0) as usize, "Whisper"))?;
    let mut dagit: Vec<Vec<whisper::Kelime>> = vec![Vec::new(); klipler.len()];
    for (a, b, s) in kel {
        let k = ofs.partition_point(|&o| o <= a + 1e-6).saturating_sub(1);
        let (o, d) = (ofs[k], klipler[k].sure);
        dagit[k].push(((a - o).clamp(0.0, d), (b - o).clamp(0.0, d), s));
    }
    Ok(dagit.iter().enumerate().flat_map(|(k, kel)| cumle_yap(kel, k)).collect())
}

fn metin_parca(w: &mut whisper::Whisper, vad: &mut vad::Vad, klipler: &[Klip], ad: &str) -> Result<Vec<Cumle>> {
    durum::toplam(ad, klipler.len());
    let mut out = Vec::new();
    for (k, kl) in klipler.iter().enumerate() {
        if kl.sure >= 0.25 {
            let x16 = ses::yeniden_ornekle(&kl.x48, SR, 16000);
            let kel = w.yaziya_dok(&x16, vad, |_| {})?;
            out.extend(cumle_yap(&kel, k));
        }
        durum::ilerle(ad, k + 1, &format!("replik {}", kl.no));
    }
    Ok(out)
}

/// Cümle cümle duygu (emotion2vec+) ve İngilizce ses gücü (Türkçe ses buna eşlenir)
fn ozellikler(c: &mut [Cumle], klipler: &[Klip], mut d: Option<&mut duygu::Duygu>) {
    for z in c.iter_mut() {
        let x = &klipler[z.klip].x48;
        let (a, b) = (((z.start * SR as f64) as usize).min(x.len()), ((z.end * SR as f64) as usize).min(x.len()));
        let parca = &x[a..b.max(a)];
        z.rms = ses::etkin_rms(parca, SR);
        let x16 = ses::yeniden_ornekle(parca, SR, 16000);
        let (duygu, p) = match d.as_deref_mut() {
            Some(m) => m.tahmin(&x16).unwrap_or(("notr".into(), 1.0)),
            None => ("notr".into(), 1.0),
        };
        z.duygu = duygu;
        z.duygu_p = p;
    }
}

// ------------------------------------------------------------------ 4–5. ses izi ve seslendirme
/// Karakterin en uzun repliklerinden ~12 sn'lik referans, 24 kHz (uzun replikten en çok kalan kadarı alınır)
fn referans_sesi(klipler: &[Klip]) -> Vec<f32> {
    let mut sira: Vec<usize> = (0..klipler.len()).filter(|&i| klipler[i].sure >= 0.8).collect();
    if sira.is_empty() {
        sira = (0..klipler.len()).collect();
    }
    sira.sort_by(|&a, &b| klipler[b].sure.total_cmp(&klipler[a].sure));
    let mut o = Vec::new();
    for i in sira {
        let kalan = REFERANS_SN - o.len() as f64 / 24000.0;
        if kalan < 1.0 {
            break;
        }
        let x = ses::sessizligi_kirp(&ses::yeniden_ornekle(&klipler[i].x48, SR, 24000), 24000);
        let n = x.len().min((kalan * 24000.0) as usize);
        o.extend_from_slice(&x[..n]);
        o.extend(std::iter::repeat_n(0.0f32, (0.15 * 24000.0) as usize));
    }
    if o.len() < 36000 {
        o.resize(36000, 0.0);
    }
    o
}

/// Bir cümle → 44,1 kHz mono (Evrenot seslendirme.rs ile aynı denetimler: süre ve kadın / erkek)
/// Türkçe ses modeli: ZONOS3 (ses izi) ya da VoxCPM2 (referans WAV, adım sayısı)
#[derive(Clone, Copy)]
enum Motor<'a> {
    Zonos(&'a [f32]),
    Vox(&'a Path, u32),
    /// Drama 3: referans ses ve metni; duygu yerine oyunculuk yönü (d.0)
    Drama(&'a Path, &'a str),
}

fn seslendir(say: &str, motor: Motor, tohum: u64, kadin: bool, d: Option<(&str, f32)>, cins: &mut konusmaci::Cinsiyet) -> Result<Option<Vec<f32>>> {
    let soz = trnorm::okunus(say.trim());
    if soz.trim().is_empty() {
        return Ok(None);
    }
    let h = cumle::hece(&soz).max(2) as f64;
    let beklenen = h / 6.0;
    let en_uzun = (h / 3.0).max(1.5) * 1.8 + 2.0;
    let mut yedek: Option<(f32, Vec<f32>)> = None;
    for deneme in 0..3u64 {
        let t = tohum + deneme * 104_729;
        let sonuc = match motor {
            Motor::Zonos(iz) => zonos::oku(&soz, Some(iz), t, beklenen, d),
            Motor::Vox(r, adim) => vox::oku(&soz, r, t, adim, d.map(|x| x.0)),
            Motor::Drama(r, m) => drama::oku(&soz, r, m, t, d.map(|x| x.0)),
        };
        let (a, sr) = match sonuc {
            Ok(x) => x,
            Err(e) => {
                gunluk(&format!("ses modeli deneme {}: {e:#}", deneme + 1));
                continue;
            }
        };
        let a = ses::sessizligi_kirp(&ses::yeniden_ornekle(&a, sr, miks::SR), miks::SR);
        let sure = a.len() as f64 / miks::SR as f64;
        if sure < 0.15 || sure > en_uzun {
            continue;
        }
        if a.len() >= 22050 {
            let x16 = ses::yeniden_ornekle(&a, miks::SR, 16000);
            if let Ok(pf) = cins.kadin_uzun(&x16) {
                let ters = if kadin { 1.0 - pf } else { pf };
                if ters >= TERS {
                    if yedek.as_ref().is_none_or(|(b, _)| ters < *b) {
                        yedek = Some((ters, a));
                    }
                    continue;
                }
            }
        }
        return Ok(Some(a));
    }
    Ok(yedek.map(|(_, a)| a))
}

/// Replik replik konuşma dili (Whisper dil belirleme). Önbellek: calisma/<karakter>/diller.json
fn diller_al(y: &Yollar, calisma: &Path, en_dizin: &Path, n: usize) -> Result<Vec<(String, f32)>> {
    let onb = calisma.join("diller.json");
    if let Some(d) = json_oku::<Vec<(String, f32)>>(&onb)
        && d.len() == n
    {
        return Ok(d);
    }
    let ad = "Dil belirleme (Whisper)";
    durum::basla(ad, n);
    onnx::hazirla(&y.bin)?;
    let mut w = whisper::Whisper::ac(&y.modeller.join("whisper-onnx")).context("Whisper açılamadı")?;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let s = ses::wav_oku(&en_dizin.join(format!("{:04}.wav", i + 1)))?;
        let x16 = ses::yeniden_ornekle(&s.mono(), s.sr, 16000);
        let d = if x16.len() < 8000 { ("en".to_string(), 0.0) } else { w.dil(&x16).unwrap_or(("en".to_string(), 0.0)) };
        out.push(d);
        if i % 20 == 0 || i + 1 == n {
            durum::ilerle(ad, i + 1, &format!("replik {}", i + 1));
        }
    }
    json_yaz(&onb, &out);
    durum::bitir(ad);
    Ok(out)
}

// ------------------------------------------------------------------ v2: Drama 3 referansı, Qwen3-Omni yönleri
/// Cümlenin oyunculuk yönü anahtarı: bankadaki replik no + cümle başlangıcı
fn yon_anahtar(no: usize, start: f64) -> String {
    format!("{no}:{start:.2}")
}

fn notr_yon(v: &str) -> bool {
    let t = v.trim().trim_matches(['.', '"', '\'', '[', ']']).to_lowercase();
    t.is_empty() || t == "neutral" || t == "none" || t.starts_with("neutral")
}

/// Drama 3 klonlaması referans ses + metni ister: karakterin uzun bir repliğinin baştan ~15 sn'ye kadar olan tam
/// cümleleri (sesi o cümlelerin sonunda kesilir, metni Whisper'dan). Bir kez seçilir, sonra hep aynısı kullanılır.
fn drama_referansi(klipler: &[Klip], c: &[Cumle], calisma: &Path) -> Result<(PathBuf, String)> {
    let wav = calisma.join("referans_drama.wav");
    let txt = calisma.join("referans_drama.txt");
    if wav.is_file()
        && let Ok(t) = std::fs::read_to_string(&txt)
        && !t.trim().is_empty()
    {
        return Ok((wav, t.trim().to_string()));
    }
    let mut en_iyi: Option<(f64, usize, f64, f64, String)> = None;
    for (k, kl) in klipler.iter().enumerate() {
        let mut bu: Vec<&Cumle> = c.iter().filter(|z| z.klip == k).collect();
        bu.sort_by(|a, b| a.start.total_cmp(&b.start));
        let Some(ilk) = bu.first() else { continue };
        let bas = (ilk.start - 0.15).max(0.0);
        let mut son = None;
        let mut metin = Vec::new();
        for z in &bu {
            if z.end - bas > 15.0 {
                break;
            }
            son = Some(z.end);
            metin.push(z.en.trim().to_string());
        }
        let Some(son) = son else { continue };
        let bit = (son + 0.15).min(kl.sure);
        let sure = bit - bas;
        if en_iyi.as_ref().is_none_or(|e| sure > e.0) {
            en_iyi = Some((sure, k, bas, bit, metin.join(" ")));
        }
    }
    // kısa konuşan yan karakterlerde (3 replik) uzun replik olmayabilir: en uzunu alınır
    let (sure, k, bas, bit, metin) = en_iyi.context("Drama 3 için referans replik bulunamadı")?;
    if sure < 1.0 {
        bail!("Drama 3 için referans replik çok kısa ({sure:.1} sn)");
    }
    let x = &klipler[k].x48;
    let (a, b) = (((bas * SR as f64) as usize).min(x.len()), ((bit * SR as f64) as usize).min(x.len()));
    ses::wav_yaz_mono(&wav, &x[a..b.max(a)], SR)?;
    std::fs::write(&txt, &metin)?;
    gunluk(&format!("Drama 3 referansı: replik {} ({:.1} sn)", klipler[k].no, bit - bas));
    Ok((wav, metin))
}

/// Cümle cümle oyunculuk yönü (Qwen3-Omni, İngilizce sesi dinleyerek): önbellekte olmayanların sesleri 16 kHz
/// WAV olarak yazılır, linux/v2/yon_cikar.py hepsini sunucuya sorar. Önbellek: calisma/<karakter>/yonler_omni.json
fn yonleri_al(y: &Yollar, calisma: &Path, klipler: &[Klip], c: &[Cumle]) -> Result<HashMap<String, String>> {
    let onb = calisma.join("yonler_omni.json");
    let mut yonler: HashMap<String, String> = json_oku(&onb).unwrap_or_default();
    let dizin = calisma.join("yon");
    std::fs::create_dir_all(&dizin)?;
    let mut gorev = Vec::new();
    for z in c {
        let no = klipler[z.klip].no;
        let anahtar = yon_anahtar(no, z.start);
        if yonler.contains_key(&anahtar) || z.en.trim().is_empty() {
            continue;
        }
        let x = &klipler[z.klip].x48;
        let a = (((z.start - 0.1).max(0.0) * SR as f64) as usize).min(x.len());
        let b = (((z.end + 0.1) * SR as f64) as usize).min(x.len());
        if b <= a {
            continue;
        }
        let p = dizin.join(format!("{no:04}_{:06}.wav", (z.start * 100.0).round() as u64));
        ses::wav_yaz_mono(&p, &ses::yeniden_ornekle(&x[a..b], SR, 16000), 16000)?;
        gorev.push(serde_json::json!({"anahtar": anahtar, "wav": p.to_string_lossy(), "en": z.en.trim()}));
    }
    if gorev.is_empty() {
        return Ok(yonler);
    }
    let gorev_yol = calisma.join("yon_gorev.json");
    json_yaz(&gorev_yol, &gorev);
    dis::komut(y, &["baslat", "omni"])?;
    let (g, o) = (gorev_yol.to_string_lossy().to_string(), onb.to_string_lossy().to_string());
    let sonuc = dis::komut(y, &["yon", g.as_str(), o.as_str()]);
    let _ = dis::komut(y, &["durdur", "omni"]);
    sonuc?;
    yonler = json_oku(&onb).unwrap_or_default();
    Ok(yonler)
}

// ------------------------------------------------------------------ 6. yerleştirme ve stüdyo miksi
struct Dublaj {
    /// replik başına çıkış (48 kHz); None = Türkçe yok, orijinal kalır
    sesler: Vec<Option<Vec<f32>>>,
    /// cümle başına (başlangıç, bitiş, hız katı), replik içinde
    yerler: Vec<Option<(f64, f64, f32)>>,
}

fn yerlestir_ve_isle(kip: Kip, klipler: &[Klip], c: &[Cumle], sesler: &[Option<Vec<f32>>], ayar: &Ayar, gecici: &Path, ad: &str) -> Result<Dublaj> {
    let mut yerler: Vec<Option<(f64, f64, f32)>> = vec![None; c.len()];
    // önce her replik kendi içinde: cümleler İngilizce yerlerinde (sığmazsa en çok 1,35 kat hızlanır)
    let mut ham: Vec<Option<Vec<f32>>> = Vec::with_capacity(klipler.len());
    for (k, kl) in klipler.iter().enumerate() {
        let idx: Vec<usize> = (0..c.len()).filter(|&i| c[i].klip == k).collect();
        if idx.is_empty() || idx.iter().all(|&i| sesler[i].is_none()) {
            ham.push(None);
            continue;
        }
        let s: Vec<Option<Vec<f32>>> = idx.iter().map(|&i| sesler[i].clone()).collect();
        let y: Vec<miks::Yer> = idx.iter().map(|&i| miks::Yer { start: c[i].start, end: c[i].end, rms: c[i].rms }).collect();
        let (iz, yer) = miks::yerlestir(&s, &y, kl.sure, ayar.en_cok_hiz, gecici)?;
        let son = yer.iter().flatten().map(|v| v.1).fold(0.0f64, f64::max);
        let mut d48 = ses::yeniden_ornekle(&iz, miks::SR, SR);
        let boy = ((kl.sure.max(son + 0.1)) * SR as f64) as usize;
        d48.resize(boy.max(kl.x48.len()), 0.0);
        for (j, &i) in idx.iter().enumerate() {
            yerler[i] = yer[j];
        }
        ham.push(Some(d48));
    }
    let anahtar = &ayar.studyo;
    let mut sonuc: Vec<Option<Vec<f32>>> = vec![None; klipler.len()];
    match kip {
        Kip::Sabit | Kip::Parca => {
            for (k, kl) in klipler.iter().enumerate() {
                if let Some(d) = &ham[k] {
                    let mut en = kl.x48.clone();
                    en.resize(d.len(), 0.0);
                    let idx: Vec<usize> = (0..c.len()).filter(|&i| c[i].klip == k).collect();
                    let cm: Vec<(f64, f64)> = idx.iter().map(|&i| (c[i].start, c[i].end)).collect();
                    let zm: Vec<Option<[f64; 2]>> = idx.iter().map(|&i| yerler[i].map(|(a, b, _)| [a, b])).collect();
                    let y = if anahtar.is_empty() { d.clone() } else { studyo::isle(d, &en, anahtar, &gecici.join("_studyo"), &cm, &zm)? };
                    sonuc[k] = Some(y);
                }
                durum::ilerle(ad, k + 1, &format!("replik {}", kl.no));
            }
        }
        Kip::Birlesik => {
            // bütün replikler tek iz (araları 1,5 sn), stüdyo miksi bütün izde, sonra aynı yerlerden geri bölünür
            let bosluk = (BOSLUK * SR as f64) as usize;
            let (mut dub, mut en) = (Vec::new(), Vec::new());
            let mut ofs = Vec::new();
            let (mut cm, mut zm) = (Vec::new(), Vec::new());
            for (k, kl) in klipler.iter().enumerate() {
                let o = dub.len();
                let boy = ham[k].as_ref().map_or(kl.x48.len(), |d| d.len());
                ofs.push((o, boy));
                let t0 = o as f64 / SR as f64;
                match &ham[k] {
                    Some(d) => dub.extend_from_slice(d),
                    None => dub.extend(std::iter::repeat_n(0.0f32, boy)),
                }
                en.extend_from_slice(&kl.x48);
                en.resize(o + boy, 0.0);
                for i in (0..c.len()).filter(|&i| c[i].klip == k) {
                    cm.push((c[i].start + t0, c[i].end + t0));
                    zm.push(yerler[i].map(|(a, b, _)| [a + t0, b + t0]));
                }
                dub.extend(std::iter::repeat_n(0.0f32, bosluk));
                en.extend(std::iter::repeat_n(0.0f32, bosluk));
            }
            durum::ilerle(ad, 0, &format!("stüdyo miksi {} sn'lik birleşik izde", sure_yaz(dub.len() as f64 / SR as f64)));
            let y = if anahtar.is_empty() { dub } else { studyo::isle(&dub, &en, anahtar, &gecici.join("_studyo"), &cm, &zm)? };
            for (k, &(o, boy)) in ofs.iter().enumerate() {
                if ham[k].is_some() {
                    sonuc[k] = Some(y[o.min(y.len())..(o + boy).min(y.len())].to_vec());
                }
                durum::ilerle(ad, k + 1, "geri bölünüyor");
            }
        }
    }
    Ok(Dublaj { sesler: sonuc, yerler })
}

/// Bütün replikler arka arkaya (araları 0,6 sn): baştan sona dinlemek için
fn onizleme(cikis: &Path, dizin: &Path, klipler: &[Klip]) -> Result<()> {
    let mut o = Vec::new();
    for kl in klipler {
        let s = ses::wav_oku(&dizin.join(format!("{:04}.wav", kl.no)))?;
        o.extend(ses::yeniden_ornekle(&s.mono(), s.sr, SR));
        o.extend(std::iter::repeat_n(0.0f32, (0.6 * SR as f64) as usize));
    }
    ses::wav_yaz_mono(cikis, &o, SR)
}

// ------------------------------------------------------------------ 7. karşılaştırma sayfası
/// Çıkış klasörünün adından okunur ad: "sabit" → "ZONOS3 · sabit ses izi", "sabit_voxcpm2_30" → "VoxCPM2 · 30 adım"
fn klasor_adi(d: &str) -> String {
    let kip = |k: &str| match k {
        "birlesik" => "birleşik",
        "parca" => "parça parça",
        _ => "sabit ses izi",
    };
    match d.split_once("_voxcpm2_") {
        Some((k, adim)) => match adim.strip_suffix("_duygu") {
            Some(a) => format!("VoxCPM2 · {a} adım · duygulu · {}", kip(k)),
            None => format!("VoxCPM2 · {adim} adım · duygusuz · {}", kip(k)),
        },
        None => format!("ZONOS3 · {}", kip(d)),
    }
}

/// Sayfa bu çalıştırmanın dökümünden (metinler) ve Cikti\<karakter> içindeki bütün Türkçe denemelerden (önceki
/// çalıştırmalar dahil: ZONOS3, VoxCPM2…) yapılır: her replik için hepsi yan yana dinlenir.
fn rapor(yol: &Path, k: &str, ayar: &Ayar, klipler: &[Klip], dokumler: &HashMap<Kip, Vec<serde_json::Value>>, cikti: &Path) -> Result<()> {
    use durum::esc;
    use std::fmt::Write as _;
    // denemeler: ingilizce dışındaki, bu repliklerden en az birini içeren klasörler (eskiden yeniye)
    let mut denemeler: Vec<(std::time::SystemTime, String)> = std::fs::read_dir(cikti)?
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|d| d != "ingilizce" && klipler.iter().any(|kl| cikti.join(d).join(format!("{:04}.wav", kl.no)).is_file()))
        .map(|d| (std::fs::metadata(cikti.join(&d)).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH), d))
        .collect();
    denemeler.sort();
    let mut h = String::new();
    let _ = write!(
        h,
        "<!DOCTYPE html><html lang=\"tr\"><head><meta charset=\"utf-8\"><title>{k} · Türkçe dublaj karşılaştırması</title><style>\
body{{font-family:Segoe UI,system-ui,sans-serif;background:#16181d;color:#e8e8ea;margin:0;padding:24px 32px}}\
h1{{font-size:22px;margin:0 0 6px}}.alt{{color:#9aa0aa;margin-bottom:18px}}\
.ust{{display:flex;gap:14px;flex-wrap:wrap;margin-bottom:22px}}.kart{{background:#1f2229;border-radius:12px;padding:12px 14px}}\
.kart b{{display:block;margin-bottom:6px}}audio{{height:32px;width:260px}}table{{border-collapse:collapse;width:100%}}\
td,th{{border-bottom:1px solid #2c3039;padding:8px 8px;vertical-align:top;text-align:left;font-size:13.5px}}\
th{{color:#9aa0aa;font-weight:600;position:sticky;top:0;background:#16181d}}.en{{color:#b9bec8}}.tr{{color:#fff}}\
.k{{color:#9aa0aa;font-size:12px;margin-top:4px}}.uyari{{color:#ffb86b}}td audio{{width:230px}}\
</style></head><body><h1>{k} · Türkçe dublaj karşılaştırması</h1><div class=\"alt\">{} replik · Türkçe aksan %{:.0} (yalnız ZONOS3) · en çok hız {:.2}× · stüdyo: {}</div><div class=\"ust\">",
        klipler.len(),
        ayar.aksan * 100.0,
        ayar.en_cok_hiz,
        esc(&ayar.studyo.join(", "))
    );
    let _ = write!(h, "<div class=\"kart\"><b>İngilizce (orijinal), baştan sona</b><audio controls preload=\"none\" src=\"{k}_ingilizce_onizleme.wav\"></audio></div>");
    for (_, d) in &denemeler {
        let on = format!("{k}_{d}_onizleme.wav");
        if cikti.join(&on).is_file() {
            let _ = write!(h, "<div class=\"kart\"><b>{}, baştan sona</b><audio controls preload=\"none\" src=\"{}\"></audio></div>", esc(&klasor_adi(d)), esc(&on));
        }
    }
    let _ = write!(h, "</div><table><tr><th>No</th><th>İngilizce</th><th>Türkçe</th><th>Dinle</th></tr>");
    let bul = |x: Kip, no: usize| dokumler.get(&x).and_then(|d| d.iter().find(|v| v["no"].as_u64() == Some(no as u64)).cloned());
    for kl in klipler {
        let mut en = String::new();
        let mut tr = String::new();
        let mut ek = String::new();
        let ana = bul(Kip::Sabit, kl.no).or_else(|| bul(Kip::Birlesik, kl.no)).or_else(|| bul(Kip::Parca, kl.no));
        if let Some(v) = &ana {
            for c in v["cumleler"].as_array().into_iter().flatten() {
                let _ = write!(en, "{} ", esc(c["en"].as_str().unwrap_or("")));
                let _ = write!(tr, "{} ", esc(c["tr"].as_str().unwrap_or("")));
                let (d, p) = (c["duygu"].as_str().unwrap_or(""), c["duygu_p"].as_f64().unwrap_or(0.0));
                if duygu::belirgin(d, p as f32) {
                    let ad = match d {
                        "kizgin" => "kızgın",
                        "uzgun" => "üzgün",
                        "mutlu" => "mutlu",
                        "korkmus" => "korkmuş",
                        "igrenmis" => "iğrenmiş",
                        "saskin" => "şaşkın",
                        x => x,
                    };
                    let _ = write!(ek, "<div class=\"k\">duygu: {ad} (%{:.0}) · {}</div>", p * 100.0, esc(&kisalt(c["en"].as_str().unwrap_or(""), 40)));
                }
                if let Some(f) = c["yer"][2].as_f64()
                    && f > 1.02
                {
                    let _ = write!(ek, "<div class=\"k uyari\">bir cümle {f:.2}× hızlandı</div>");
                }
            }
            if v["orijinal_kaldi"].as_bool() == Some(true) {
                ek.push_str("<div class=\"k\">konuşma yok / okunamadı: orijinal ses</div>");
            }
            let (se, st) = (v["sure_en"].as_f64().unwrap_or(0.0), v["sure_tr"].as_f64().unwrap_or(0.0));
            let _ = write!(ek, "<div class=\"k\">süre {se:.1} sn → {st:.1} sn (bu deneme)</div>");
        }
        let mut dinle = format!("<div class=\"k\">İngilizce</div><audio controls preload=\"none\" src=\"ingilizce/{:04}.wav\"></audio>", kl.no);
        for (_, d) in &denemeler {
            let f = cikti.join(d).join(format!("{:04}.wav", kl.no));
            if f.is_file() {
                let sure = ses::wav_oku(&f).map(|s| s.uzunluk() as f64 / s.sr as f64).unwrap_or(0.0);
                let _ = write!(
                    dinle,
                    "<div class=\"k\">{} · {sure:.1} sn</div><audio controls preload=\"none\" src=\"{}/{:04}.wav\"></audio>",
                    esc(&klasor_adi(d)),
                    esc(d),
                    kl.no
                );
            }
        }
        let _ = write!(h, "<tr><td>{}</td><td class=\"en\">{en}</td><td class=\"tr\">{tr}{ek}</td><td>{dinle}</td></tr>", kl.no);
    }
    h.push_str("</table></body></html>");
    std::fs::write(yol, h)?;
    Ok(())
}

// ------------------------------------------------------------------ oyuna paketleme
/// Deneme cümlesi: paketlerken Türkçesi henüz olmayan bütün repliklerin yerine (oyunda karakter ne zaman konuşsa duyulur)
const DENEME_METNI: &str = "Merhaba! Bu ses, Forza Horizon altı için Türkçe dublaj denemesidir.";

/// Karakterin bankasını yeniden yapar: Türkçesi olan replikler Türkçe, olmayanlar İngilizce (ya da --deneme ile deneme
/// cümlesi). Çıktı: Cikti\<karakter>\paket\<banka adı>. Oyuna kopyalamak ayrı (Oyuna_kur.bat).
fn paketle(y: &Yollar, ayar: &Ayar) -> Result<()> {
    let k = ayar.karakter.clone();
    let calisma = y.calisma.join(&k);
    let en_dizin = calisma.join("en");
    std::fs::create_dir_all(&en_dizin)?;
    let cikti = y.proje.join("Cikti").join(&k);
    let paket_dizin = cikti.join("paket");
    std::fs::create_dir_all(&paket_dizin)?;
    let etiket = if ayar.model == "drama3" {
        format!("sabit_drama3{}", if ayar.duygu { "_duygu" } else { "" })
    } else if ayar.model == "voxcpm2" {
        format!("sabit_voxcpm2_{}{}", ayar.adim, if ayar.duygu { "_duygu" } else { "" })
    } else {
        "sabit".to_string()
    };
    let tr_dizin = cikti.join(&etiket);

    let banka_yolu = banka_bul(y, &k)?;
    let banka_adi = banka_yolu.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let orijinal = std::fs::read(&banka_yolu)?;
    let pk = fsb::ayikla(&orijinal)?;
    let n = pk.parcalar.len();

    let ad_en = "1. İngilizce sesler (vgmstream)".to_string();
    let ad_deneme = "2. Deneme cümlesi (VoxCPM2)".to_string();
    let ad_paket = "3. Ses bankası yapılıyor".to_string();
    let ad_kontrol = "4. Kontrol".to_string();
    let mut asamalar = vec![(ad_en.clone(), 3.0 + 0.03 * n as f64), ("Dil belirleme (Whisper)".to_string(), 10.0 + 0.15 * n as f64)];
    if ayar.deneme {
        asamalar.push((ad_deneme.clone(), 40.0));
    }
    asamalar.push((ad_paket.clone(), 5.0 + 0.01 * n as f64));
    asamalar.push((ad_kontrol.clone(), 5.0));
    let html = y.kok.join("durum.html");
    durum::kur(&format!("{k} · oyun için paketleme"), html.clone(), calisma.join("paket_gunluk.txt"), &asamalar);
    tarayicida_ac(&html);
    gunluk(&format!("{banka_adi}: {n} replik; Türkçe sesler: {}", tr_dizin.display()));

    let hepsi: Vec<usize> = (0..n).collect();
    sesleri_cikar(y, &pk, &hepsi, &en_dizin, &ad_en)?;

    // İngilizce bankada da Japonca (ya da başka dilde) konuşan replikler var: oyunun İngilizce sürümünde de
    // orijinal dilde kalıyorlar, Türkçe sürümde de orijinal kalsınlar. Dil Whisper'la bir kez belirlenir
    // (calisma/<karakter>/diller.json), İngilizce olmayan repliklere Türkçe ses konmaz.
    let diller = diller_al(y, &calisma, &en_dizin, n)?;
    let yabanci: HashSet<usize> = diller
        .iter()
        .enumerate()
        .filter(|(_, (d, p))| d != "en" && *p >= 0.6)
        .map(|(i, _)| i)
        .collect();
    if !yabanci.is_empty() {
        let mut say: HashMap<&str, usize> = HashMap::new();
        for &i in &yabanci {
            *say.entry(diller[i].0.as_str()).or_default() += 1;
        }
        gunluk(&format!("İngilizce olmayan {} replik orijinal kalacak: {:?}", yabanci.len(), say));
    }

    // deneme cümlesi: karakterin sabit referans sesiyle, VoxCPM2
    let mut deneme: Option<Vec<f32>> = None;
    if ayar.deneme {
        durum::basla(&ad_deneme, 1);
        let ref_yol = calisma.join("referans.wav");
        if !ref_yol.is_file() {
            bail!("{} yok: önce bu karakterin dublajını çalıştırın", ref_yol.display());
        }
        onnx::hazirla(&y.bin)?;
        let mut cins = konusmaci::Cinsiyet::ac(&y.modeller.join("cinsiyet/model_quantized.onnx"))?;
        let kadin = match json_oku::<IzKaydi>(&calisma.join("ses_izi.json")) {
            Some(x) => x.kadin,
            None => {
                let r = ses::wav_oku(&ref_yol)?;
                cins.kadin_uzun(&ses::yeniden_ornekle(&r.mono(), r.sr, 16000)).unwrap_or(0.5) >= 0.5
            }
        };
        durum::ilerle(&ad_deneme, 0, "VoxCPM2 yükleniyor");
        let mut vox = vox::Vox::new(y);
        vox.baslat()?;
        let a = seslendir(DENEME_METNI, Motor::Vox(&ref_yol, ayar.adim), TOHUM, kadin, None, &mut cins)?.context("deneme cümlesi seslendirilemedi")?;
        vox.durdur();
        let mut a = ses::yeniden_ornekle(&a, miks::SR, SR);
        // ses gücü karakterin İngilizce repliklerine yakın olsun
        let ref_rms = {
            let r = ses::wav_oku(&en_dizin.join("0001.wav"))?;
            ses::etkin_rms(&r.mono(), r.sr)
        };
        let o = ses::etkin_rms(&a, SR);
        if o > 0.0 && ref_rms > 0.0 {
            let kat = (ref_rms / o).clamp(0.25, 4.0);
            a.iter_mut().for_each(|v| *v *= kat);
        }
        ses::wav_yaz_mono(&paket_dizin.join("deneme_cumlesi.wav"), &a, SR)?;
        gunluk(&format!("deneme cümlesi {:.1} sn: \"{DENEME_METNI}\"", a.len() as f64 / SR as f64));
        durum::bitir(&ad_deneme);
        deneme = Some(a);
    }

    durum::basla(&ad_paket, n);
    let gecici_paket = calisma.join("gecici");
    std::fs::create_dir_all(&gecici_paket)?;
    let (mut sayi_tr, mut sayi_deneme, mut sayi_en) = (0usize, 0usize, 0usize);
    let mut sesler: Vec<Vec<i16>> = Vec::with_capacity(n);
    for i in 0..n {
        let ad = format!("{:04}.wav", i + 1);
        let tr = tr_dizin.join(&ad);
        let opus = tr.with_extension("opus");
        let x = if yabanci.contains(&i) {
            sayi_en += 1;
            let s = ses::wav_oku(&en_dizin.join(&ad))?;
            ses::yeniden_ornekle(&s.mono(), s.sr, SR)
        } else if tr.is_file() {
            sayi_tr += 1;
            let s = ses::wav_oku(&tr)?;
            ses::yeniden_ornekle(&s.mono(), s.sr, SR)
        } else if opus.is_file() {
            // buluttan gelen sıkıştırılmış Türkçe ses
            sayi_tr += 1;
            let g = gecici_paket.join("_opus.wav");
            ses::ffmpeg(&["-i", &opus.to_string_lossy(), "-ar", "48000", "-ac", "1", &g.to_string_lossy()])?;
            let s = ses::wav_oku(&g)?;
            ses::yeniden_ornekle(&s.mono(), s.sr, SR)
        } else if let Some(d) = &deneme {
            sayi_deneme += 1;
            d.clone()
        } else {
            sayi_en += 1;
            let s = ses::wav_oku(&en_dizin.join(&ad))?;
            ses::yeniden_ornekle(&s.mono(), s.sr, SR)
        };
        sesler.push(paket::i16e(&x));
        if i % 20 == 0 || i + 1 == n {
            durum::ilerle(&ad_paket, i + 1, &format!("replik {}", i + 1));
        }
    }
    let yeni_fsb = paket::fsb_pcm(&pk.bayt, &sesler)?;
    let yeni = paket::banka_yap(&orijinal, pk.konum, pk.bayt.len(), &yeni_fsb)?;
    gunluk(&format!(
        "{sayi_tr} replik Türkçe, {sayi_deneme} replik deneme cümlesi, {sayi_en} replik İngilizce; banka {:.0} MB → {:.0} MB",
        orijinal.len() as f64 / 1e6,
        yeni.len() as f64 / 1e6
    ));
    durum::bitir(&ad_paket);

    // kontrol: paket yeniden okunur (replik sayısı, süreler) ve vgmstream ilk Türkçe repliği çözebiliyor mu
    durum::basla(&ad_kontrol, 2);
    let q = fsb::ayikla(&yeni)?;
    if q.parcalar.len() != n {
        bail!("kontrol: replik sayısı tutmuyor ({} ≠ {n})", q.parcalar.len());
    }
    for (i, s) in sesler.iter().enumerate() {
        if (q.sure(i) - s.len() as f64 / SR as f64).abs() > 1e-6 {
            bail!("kontrol: replik {} süresi tutmuyor", i + 1);
        }
    }
    durum::ilerle(&ad_kontrol, 1, "vgmstream ile çözme denemesi");
    let deneme_no = (0..n).find(|&i| tr_dizin.join(format!("{:04}.wav", i + 1)).is_file()).unwrap_or(0);
    let gecici = calisma.join("gecici");
    std::fs::create_dir_all(&gecici)?;
    let (fsb_yol, wav_yol) = (gecici.join("_kontrol.fsb"), gecici.join("_kontrol.wav"));
    std::fs::write(&fsb_yol, &yeni_fsb)?;
    let _ = std::fs::remove_file(&wav_yol);
    let mut c = std::process::Command::new(y.vgmstream());
    c.args(["-i", "-s", &(deneme_no + 1).to_string(), "-o"]).arg(&wav_yol).arg(&fsb_yol);
    ses::pencere_acma(&mut c);
    let _ = c.output();
    let cozuldu = ses::wav_oku(&wav_yol).map(|s| s.uzunluk() == sesler[deneme_no].len()).unwrap_or(false);
    let _ = std::fs::remove_file(&fsb_yol);
    if !cozuldu {
        bail!("kontrol: vgmstream yeni paketteki replik {}'i çözemedi", deneme_no + 1);
    }
    gunluk(&format!("kontrol tamam: {n} replik, vgmstream replik {}'i doğru uzunlukta çözdü", deneme_no + 1));
    let cikis = paket_dizin.join(&banka_adi);
    std::fs::write(&cikis, &yeni)?;
    durum::bitir(&ad_kontrol);
    gunluk(&format!("Hazır: {}", cikis.display()));
    let adres = format!("file:///{}", paket_dizin.to_string_lossy().replace('\\', "/").replace(' ', "%20"));
    durum::tamam("Paket klasörünü aç", &adres);
    Ok(())
}
