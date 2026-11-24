# Forza Horizon 6 — Türkçe Dublaj (yapay zekâ ile)

Forza Horizon 6'daki İngilizce konuşmaları **Türkçe dublajla** değiştiren hayran yapımı bir mod.
Karakterler, 8 radyo DJ'i ve navigasyon sesi Türkçe konuşur. Steam ve Xbox / Game Pass sürümleriyle çalışır.



https://github.com/user-attachments/assets/cbd6e68f-bd32-4711-b2b2-43b2862e450f



> Resmi değildir. Seslendirme tamamen yapay zekâ ile üretilmiştir. Forza Horizon 6 ve oyun içeriği Playground Games / Microsoft'a aittir.

---

## Kurulum (oyuncular için)

1. [Releases](../../releases) sayfasından **bütün** `FH6_Turkce_Dublaj_BolumX.zip` dosyalarını indirin.
2. Hepsini **aynı klasöre** çıkarın (sonunda `FH6_Turkce_Dublaj_Kurulum` klasöründe `Kurulum.exe` ve `Veri` olmalı).
3. Oyunu kapatın, `Kurulum.exe`'yi çalıştırın, oyununuzu seçip **Türkçe Dublajı Kur**'a basın.

- **Kaldırmak için:** aynı programda **Kaldır** düğmesine basın. Orijinal İngilizce dosyalar yedekten geri gelir.
- **Xbox / Game Pass:** Dosyalara yazılamazsa Xbox uygulamasında oyunda **Yönet > Dosyalar > Gelişmiş yönetim özellikleri**'ni açın.
- **Oyun güncellenirse:** Kurulumu tekrar çalıştırın.

---

## Nasıl yaptık?

Bütün süreç bir bulut GPU sunucusunda (RunPod, NVIDIA L40S 48 GB) açık kaynak modellerle çalıştı.
Ana araç Rust ile yazdığımız `ForzaDublaj`; adımları sırayla o yönetiyor.

| Adım | Ne yapıyor | Kullanılan |
|---|---|---|
| 1. Ses dosyalarını çıkarma | Oyunun FMOD `.bank` dosyalarından replikleri WAV olarak çıkarır | vgmstream |
| 2. Yazıya dökme | İngilizce replikleri zaman damgalı metne çevirir | **Whisper large-v3-turbo** |
| 3. Dil ayıklama + oyunculuk notu | Japonca replikleri ayırır (onlar orijinal kalır); her cümle için duygu/ton notu çıkarır (kızgın, heyecanlı, fısıltı…) | **Qwen3-Omni-30B-A3B** (AWQ 4-bit) |
| 4. Çeviri | İngilizce → Türkçe | **Hy-MT2-30B-A3B** (FP8) |
| 5. Seslendirme | Karakterin kendi İngilizce sesini referans alarak, duygu notlarıyla Türkçe okur | **Fish Audio Drama 3** (vLLM-Omni ile) |
| 6. Ses modeli (ana karakterler) | Mei ve Jordan için karakterin bütün İngilizce kayıtlarıyla ses modeli eğitip Drama 3 çıktısını bu modelden geçirdik | **RVC** (Applio, RMVPE + ContentVec, 80 epoch) |
| 7. Paketleme | Yeni sesleri sürelerine uydurup oyunun `.bank` dosyalarına geri paketler | ForzaDublaj (Rust) |
| 8. Kurulum | Steam/Xbox'ı bulur, orijinalleri yedekleyip yeni dosyaları koyar | C# (.NET Framework) |

Kısaca: **Whisper → Qwen3-Omni → Hy-MT2 → Drama 3 → (RVC) → .bank**

### Ana karakterlerin metni
Mei ve Jordan gibi çok konuşan karakterlerin Türkçe metinlerini İngilizceleriyle tek tek karşılaştırıp **Claude** ile elden geçirdik (doğal, günlük Türkçe; oyun terimleri İngilizce kaldı: drift, Time Attack, Toge…).

### Öğrendiklerimiz
- **Toplu metin düzeltirken satırlar kayabilir.** Bir modele yüzlerce satırı birden verip düzelttirdiğimizde cevaplar bir satır kaydı; bazı cümlelerin yerine yandakinin çevirisi okundu. Her satırı bir kimlik numarasıyla eşleyin ve sonuçta İngilizce–Türkçe uyumunu kontrol edin.
- **Japonca replikleri ayıklayın.** Bazı karakterler yer yer Japonca konuşuyor; onları seslendirmek boşa iş. Bir DJ tamamen Japonca olduğu için orijinal bıraktık.
- **Disk:** 200 GB disk modeller + ara dosyalarla doldu. En az **250–300 GB** ayırın.
- **Ev bilgisayarı:** 30B modeller için 24–48 GB ekran kartı belleği gerekir; 8 GB'lık bir kart yetmez.
- İlk denemede yerel olarak **VoxCPM2** kullandık. **Drama 3** duygu ve doğallıkta belirgin şekilde daha iyiydi.

---

## Maliyet

| Kalem | Tutar |
|---|---|
| GPU sunucusu (RunPod L40S 48 GB) | 1,12 $ / saat |
| **Projenin tamamı** (bütün karakterler, DJ'ler, navigasyon, Mei/Jordan ses modeli ve düzeltmeler) | **≈ 25 $** |

Örnek süreler (Mei, ~80 dakika İngilizce ses, 830 klip):
- Drama 3 ile Türkçe seslendirme: ~50 dk
- RVC ses modeli eğitimi: ~35–45 dk
- Seslerin modelden geçirilmesi: ~45 dk

Bütün modeller açık kaynak; ücretli API kullanılmadı. Ödenen tek şey GPU kirası.

---

## Depodaki dosyalar

```
araclar/ForzaDublaj/   Rust kaynak kodu (çıkarma, Whisper, çeviri, seslendirme, paketleme)
araclar/ForzaDublaj/linux/   Bulut sunucusu kurulum ve çalıştırma betikleri
kurulum/               Kurulum.exe kaynak kodu (C#) ve paket hazırlama betiği
```

RVC (Applio) ayarlarımız: 40 kHz örnekleme, RMVPE perde çıkarımı, ContentVec gömme, HiFi-GAN vocoder, 80 epoch,
dönüştürmede index oranı 0,6 ve protect 0,33. Eğitim verisi: karakterin oyundaki bütün İngilizce replikleri (Mei için ~80 dk).

---

Hayran yapımıdır, ticari amaç taşımaz. Seslendirme sanatçılarına ve oyunun yapımcılarına saygıyla.
