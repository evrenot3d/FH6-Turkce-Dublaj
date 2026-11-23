//! Türkçe sesleri oyunun ses bankasına geri koyma. Bankadaki FSB5 paketi baştan yazılır: biçim 16 bit PCM
//! (FMOD'un sıkıştırmasız biçimi; çözme gerektirmez, her FMOD sürümü çalar), 48 kHz mono, replik sırası aynı.
//! Paketin başlığındaki bayraklar ve özet (hash) orijinalden aynen alınır; banka kabındaki (RIFF) boyutlar düzeltilir.
//! Her repliğin uzunluğu kendi örnek sayısından okunur: Türkçe ses İngilizceden uzun / kısa olabilir.

use anyhow::{Context, Result, bail};

const PCM16: u32 = 2;
/// FSB5 örnekleme hızı kodu: 9 = 48 kHz
const SR_48K: u64 = 9;

fn u32le(b: &[u8], o: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(b.get(o..o + 4).context("kısa veri")?.try_into()?))
}

fn yaz32(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

/// 48 kHz mono 16 bit replikler → FSB5 paketi. `eski`: orijinal paketin baytları (başlık bayrakları ve özet için)
pub fn fsb_pcm(eski: &[u8], sesler: &[Vec<i16>]) -> Result<Vec<u8>> {
    if eski.get(..4) != Some(b"FSB5") {
        bail!("orijinal paket FSB5 değil");
    }
    let surum = u32le(eski, 4)?;
    let hs = if surum == 0 { 0x40 } else { 0x3C };
    let n = sesler.len();
    // veri: her replik 32 bayta hizalı
    let mut veri: Vec<u8> = Vec::new();
    let mut basliklar: Vec<u8> = Vec::with_capacity(n * 8);
    for s in sesler {
        let konum = veri.len();
        if konum % 32 != 0 || (konum >> 5) > 0x07FF_FFFF {
            bail!("paket çok büyük");
        }
        let ornek = s.len() as u64;
        if ornek > 0x3FFF_FFFF {
            bail!("replik çok uzun");
        }
        // bit 0: ek bilgi yok · 1-4: hız · 5-6: kanal (0 = mono) · 7-33: konum/32 · 34-63: örnek sayısı
        let v: u64 = (SR_48K << 1) | (((konum as u64) >> 5) << 7) | (ornek << 34);
        basliklar.extend_from_slice(&v.to_le_bytes());
        for x in s {
            veri.extend_from_slice(&x.to_le_bytes());
        }
        while veri.len() % 32 != 0 {
            veri.push(0);
        }
    }
    let mut b = eski[..hs].to_vec();
    yaz32(&mut b, 8, n as u32);
    yaz32(&mut b, 12, basliklar.len() as u32);
    yaz32(&mut b, 16, 0); // ad tablosu yok (orijinalde de yok)
    yaz32(&mut b, 20, veri.len() as u32);
    yaz32(&mut b, 24, PCM16);
    b.extend_from_slice(&basliklar);
    b.extend_from_slice(&veri);
    Ok(b)
}

/// Orijinal banka + yeni FSB5 paketi → yeni banka. Paket aynı konumda başlar; SNDH (paket konumu / boyu),
/// "SND " parçası ve RIFF boyu yeni pakete göre yazılır.
pub fn banka_yap(eski: &[u8], fsb_konum: usize, eski_fsb_boy: usize, yeni_fsb: &[u8]) -> Result<Vec<u8>> {
    if eski.get(..4) != Some(b"RIFF") || eski.get(8..12) != Some(b"FEV ") {
        bail!("FMOD bankası değil (RIFF/FEV yok)");
    }
    if fsb_konum + eski_fsb_boy != eski.len() {
        bail!("paketten sonra başka veri var ({} + {} ≠ {}): bu banka düzeni desteklenmiyor", fsb_konum, eski_fsb_boy, eski.len());
    }
    let sndh = eski.windows(4).position(|w| w == b"SNDH").context("SNDH yok")?;
    if sndh + 20 > fsb_konum || u32le(eski, sndh + 12)? as usize != fsb_konum || u32le(eski, sndh + 16)? as usize != eski_fsb_boy {
        bail!("SNDH beklenen paket konumunu göstermiyor");
    }
    let snd = eski[sndh..fsb_konum].windows(4).rposition(|w| w == b"SND ").map(|i| i + sndh).context("SND parçası yok")?;
    let mut b = eski[..fsb_konum].to_vec();
    b.extend_from_slice(yeni_fsb);
    if b.len() % 2 == 1 {
        b.push(0);
    }
    yaz32(&mut b, sndh + 16, yeni_fsb.len() as u32);
    yaz32(&mut b, snd + 4, (fsb_konum - (snd + 8) + yeni_fsb.len()) as u32);
    let toplam = b.len() - 8;
    yaz32(&mut b, 4, toplam as u32);
    Ok(b)
}

/// -1..1 → 16 bit
pub fn i16e(x: &[f32]) -> Vec<i16> {
    x.iter().map(|v| (v.clamp(-1.0, 1.0) * 32767.0).round() as i16).collect()
}
