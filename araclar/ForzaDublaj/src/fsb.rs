//! FMOD ses bankası (.assets.bank) → içindeki FSB5 ses paketi. Paketteki her parça (subsound) bir repliktir;
//! sıra numarası oyunun kullandığı sırayla aynıdır (vgmstream'de 1'den başlar). Bu dosyada yalnız başlık okunur
//! (kaç replik, her birinin süresi); sesi çözme işini vgmstream yapar (Vorbis kurulum başlıkları onda).

use anyhow::{Context, Result, bail};

const FREKANS: [(u64, u32); 10] =
    [(1, 8000), (2, 11000), (3, 11025), (4, 16000), (5, 22050), (6, 24000), (7, 32000), (8, 44100), (9, 48000), (10, 96000)];

pub struct Paket {
    /// FSB5 paketinin baytları (bankanın içinden kesilmiş)
    pub bayt: Vec<u8>,
    /// her replik: (örnek sayısı, örnekleme hızı, kanal)
    pub parcalar: Vec<(u64, u32, u8)>,
    /// paketin bankadaki başlangıcı
    pub konum: usize,
    pub kodek: u32,
}

impl Paket {
    pub fn sure(&self, i: usize) -> f64 {
        self.parcalar.get(i).map_or(0.0, |&(n, sr, _)| if sr > 0 { n as f64 / sr as f64 } else { 0.0 })
    }
}

fn u32le(b: &[u8], o: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(b.get(o..o + 4).context("FSB5 başlığı kısa")?.try_into()?))
}

pub fn ayikla(banka: &[u8]) -> Result<Paket> {
    let konum = banka.windows(4).position(|w| w == b"FSB5").context("bankada FSB5 ses paketi yok")?;
    let b = &banka[konum..];
    let surum = u32le(b, 4)?;
    let n = u32le(b, 8)? as usize;
    let shs = u32le(b, 12)? as usize;
    let nts = u32le(b, 16)? as usize;
    let ds = u32le(b, 20)? as usize;
    let kodek = u32le(b, 24)?;
    let hs = if surum == 0 { 0x40 } else { 0x3C };
    let boy = hs + shs + nts + ds;
    if boy > b.len() {
        bail!("FSB5 paketi bankadan uzun ({boy} > {})", b.len());
    }
    let mut p = hs;
    let mut parcalar = Vec::with_capacity(n);
    for _ in 0..n {
        let v = u64::from_le_bytes(b.get(p..p + 8).context("FSB5 parça başlığı kısa")?.try_into()?);
        p += 8;
        let mut sonraki = v & 1;
        let fi = (v >> 1) & 0xF;
        let mut kanal: u8 = if (v >> 5) & 1 == 1 { 2 } else { 1 };
        let ornek = (v >> 34) & 0x3FFF_FFFF;
        let mut sr = FREKANS.iter().find(|f| f.0 == fi).map_or(0, |f| f.1);
        while sonraki == 1 {
            let c = u32le(b, p)?;
            p += 4;
            sonraki = (c & 1) as u64;
            let boy = ((c >> 1) & 0xFF_FFFF) as usize;
            let tur = c >> 25;
            if tur == 1 {
                kanal = *b.get(p).unwrap_or(&1);
            }
            if tur == 2 {
                sr = u32le(b, p)?;
            }
            p += boy;
        }
        parcalar.push((ornek, sr, kanal));
    }
    Ok(Paket { bayt: b[..boy].to_vec(), parcalar, konum, kodek })
}
