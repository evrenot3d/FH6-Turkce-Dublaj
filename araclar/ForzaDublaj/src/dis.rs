//! v2 (bulut sunucusu, Linux): büyük modeller (Hy-MT2 30B, Qwen3-Omni, Drama 3) vLLM sunucusu olarak dışarıdan
//! açılıp kapatılır: `linux/v2/sunucu.sh baslat|durdur ceviri|omni|drama`, `sunucu.sh yon <görev> <çıkış>`.
//! Sunucular kendi süreç gruplarında açılır, kapatılırken grubun tamamı (vLLM'in alt süreçleri dahil) kapanır;
//! böylece bir sonraki model ekran kartında yer bulur. Ortam değişkeni FD_V2 tanımlıysa etkindir.

use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::yollar::Yollar;

pub fn etkin() -> bool {
    std::env::var_os("FD_V2").is_some()
}

pub fn komut(y: &Yollar, args: &[&str]) -> Result<()> {
    let betik = y.kok.join("linux").join("v2").join("sunucu.sh");
    let durum = Command::new("bash")
        .arg(&betik)
        .args(args)
        .status()
        .with_context(|| format!("{} çalıştırılamadı", betik.display()))?;
    if !durum.success() {
        bail!("sunucu.sh {} başarısız (ForzaDublaj/sunucu_*.log)", args.join(" "));
    }
    Ok(())
}
