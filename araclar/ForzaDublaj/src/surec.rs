//! Yardımcı sunucu süreçleri (audio.cpp, llama.cpp, ZONOS3): başlatma, hazır olmasını bekleme, kapatma.
//! Hepsi Windows iş nesnesine bağlanır (`cocuk.rs`): program nasıl kapanırsa kapansın onlar da kapanır.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

pub struct Surec {
    pub ad: &'static str,
    exe: PathBuf,
    gunluk: PathBuf,
    saglik: String,
    cocuk: Option<Child>,
    pub imza: String,
}

impl Surec {
    pub fn new(ad: &'static str, exe: PathBuf, gunluk: PathBuf, saglik: String) -> Self {
        Surec { ad, exe, gunluk, saglik, cocuk: None, imza: String::new() }
    }

    pub fn var_mi(&self) -> bool {
        self.exe.is_file()
    }

    pub fn calisiyor(&mut self) -> bool {
        matches!(self.cocuk.as_mut().map(|c| c.try_wait()), Some(Ok(None)))
    }

    /// Süreç açık ve aynı ayarlarla (imza) başlatıldıysa bir şey yapmaz; değilse (yeniden) başlatır ve hazır olmasını bekler
    pub fn baslat(&mut self, args: &[OsString], imza: &str, bekle: Duration) -> Result<()> {
        if self.calisiyor() && self.imza == imza {
            return Ok(());
        }
        self.durdur();
        if !self.exe.is_file() {
            bail!("{} bulunamadı: {}", self.ad, self.exe.display());
        }
        let log = std::fs::File::create(&self.gunluk)?;
        let mut c = Command::new(&self.exe);
        c.args(args).stdout(Stdio::from(log.try_clone()?)).stderr(Stdio::from(log)).stdin(Stdio::null());
        if let Some(d) = self.exe.parent() {
            c.current_dir(d);
        }
        crate::ses::pencere_acma(&mut c);
        #[cfg(windows)]
        {
            // ekran kartını besleyen sunucular önce gelsin (arka plandaki ses ayırma onları yavaşlatmasın)
            use std::os::windows::process::CommandExt;
            c.creation_flags(0x0800_0000 | 0x0000_8000); // CREATE_NO_WINDOW | ABOVE_NORMAL_PRIORITY_CLASS
        }
        let s = c.spawn().with_context(|| format!("{} başlatılamadı", self.ad))?;
        crate::cocuk::bagla(&s);
        self.cocuk = Some(s);
        self.imza = imza.to_string();
        let t0 = Instant::now();
        let ajan = ajan(Duration::from_secs(2));
        while t0.elapsed() < bekle {
            if !self.calisiyor() {
                self.imza.clear();
                bail!("{} kapandı ({})", self.ad, self.gunluk.display());
            }
            if ajan.get(&self.saglik).call().is_ok_and(|r| r.status().as_u16() < 400) {
                eprintln!("[{}] hazır ({:.1} sn)", self.ad, t0.elapsed().as_secs_f32());
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(300));
        }
        self.durdur();
        bail!("{} zamanında açılmadı ({})", self.ad, self.gunluk.display())
    }

    pub fn durdur(&mut self) {
        if let Some(mut c) = self.cocuk.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        self.imza.clear();
    }
}

impl Drop for Surec {
    fn drop(&mut self) {
        self.durdur();
    }
}

/// Yerel sunuculara istek: hata kodları hata sayılmaz (gövde okunur), zaman aşımı verilir
pub fn ajan(zaman: Duration) -> ureq::Agent {
    ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(zaman)).build().into()
}

/// Sunucu yanıtı: 400 ve üstü ise gövdedeki ilk 300 karakterle hata
pub fn yanit_al(ad: &str, mut r: ureq::http::Response<ureq::Body>) -> Result<Vec<u8>> {
    let kod = r.status().as_u16();
    let b = r.body_mut().with_config().limit(1 << 30).read_to_vec()?;
    if kod >= 400 {
        bail!("{ad} ({kod}): {}", String::from_utf8_lossy(&b).chars().take(300).collect::<String>());
    }
    Ok(b)
}

pub fn yol_str(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}
