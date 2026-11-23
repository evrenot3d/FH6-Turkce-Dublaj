//! Klasörler. Program kökü: exe'nin bulunduğu klasörden yukarı doğru içinde `Cargo.toml` olan ilk klasör
//! (ForzaDublaj). Modeller ve yardımcı programlar önce kendi `modeller/` ve `bin/` klasörlerinde aranır
//! (kurulum.bat oraya kopyalar); yoksa Evrenot'un klasöründekiler kullanılır.

use std::path::{Path, PathBuf};

/// Evrenot'un kurulu olduğu yer (modeller oradan kopyalandı)
pub const EVRENOT: &str = r"D:\EVREN PROGRAMLAR\EvrenChrome";

#[derive(Clone, Debug)]
pub struct Yollar {
    /// ForzaDublaj klasörü (günlükler buraya yazılır)
    pub kok: PathBuf,
    pub modeller: PathBuf,
    pub bin: PathBuf,
    /// ara dosyalar (çıkarılan sesler, önbellek)
    pub calisma: PathBuf,
    /// "Forza 6 Seslendirme" klasörü (Orijinal_EN burada, sonuçlar Cikti'ya)
    pub proje: PathBuf,
}

impl Yollar {
    pub fn bul() -> anyhow::Result<Self> {
        let exe = std::env::current_exe()?;
        let kok = exe
            .ancestors()
            .find(|p| p.join("Cargo.toml").is_file())
            .map(Path::to_path_buf)
            .or_else(|| exe.parent().map(Path::to_path_buf))
            .ok_or_else(|| anyhow::anyhow!("program klasörü bulunamadı"))?;
        let evrenot = PathBuf::from(EVRENOT);
        let modeller = if kok.join("modeller").join("zonos3").is_dir() { kok.join("modeller") } else { evrenot.join("modeller") };
        let bin = if kok.join("bin").join("zonos3").is_dir() { kok.join("bin") } else { evrenot.join("bin") };
        let proje = kok.parent().map(Path::to_path_buf).unwrap_or_else(|| kok.clone());
        let y = Yollar { calisma: kok.join("calisma"), modeller, bin, proje, kok };
        std::fs::create_dir_all(&y.calisma)?;
        Ok(y)
    }

    pub fn vgmstream(&self) -> PathBuf {
        self.kok.join("vgmstream").join(exe("vgmstream-cli"))
    }
}

/// Program adı: Windows'ta ".exe" ekli, Linux'ta (bulut sunucusu) eksiz
pub fn exe(ad: &str) -> String {
    if cfg!(windows) { format!("{ad}.exe") } else { ad.to_string() }
}
