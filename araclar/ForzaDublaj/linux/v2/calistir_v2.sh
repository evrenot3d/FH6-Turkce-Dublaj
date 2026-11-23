#!/usr/bin/env bash
# v2 dublaj: Whisper (önbellekteyse atlanır) → Hy-MT2 30B çeviri → Qwen3-Omni oyunculuk yönleri → Drama 3 seslendirme
# → yerleştirme + stüdyo miksi. Sonra Türkçe sesler Opus'a sıkıştırılıp sonuc/<karakter>_v2.zip yapılır.
# Kullanım: bash ForzaDublaj/linux/v2/calistir_v2.sh Mei [Jordan ...]
# Ek seçenekler FD_EK ortam değişkeniyle (ör. FD_EK="--adet 10" ile en uzun 10 replik)
set -uo pipefail
KOK="$(cd "$(dirname "$0")/../.." && pwd)"
PROJE="$(dirname "$KOK")"
cd "$KOK"
. "$HOME/.cargo/env" 2>/dev/null || true
NV="$(python3 -c 'import glob,os,site;d=[p for s in site.getsitepackages() for p in glob.glob(os.path.join(s,"nvidia","*","lib"))];print(":".join(d))')"
export LD_LIBRARY_PATH="$KOK/bin/ort:$NV:${LD_LIBRARY_PATH:-}"
export FD_V2=1 FD_CEVIRI_MODEL=hymt2
mkdir -p "$PROJE/sonuc"
G="$KOK/toplu_gunluk_v2.txt"
echo "basladi $(date)" >> "$G"
[ $# -eq 0 ] && set -- Mei

for K in "$@"; do
  echo "$K basladi $(date +%H:%M:%S)" | tee -a "$G"
  # shellcheck disable=SC2086
  if ! ./forzadublaj --toplu --karakter "$K" --dakika 0 --kip sabit --model drama3 ${FD_EK:-}; then
    echo "$K HATA $(date +%H:%M:%S) (calisma/$K/gunluk.txt)" | tee -a "$G"
    bash "$KOK/linux/v2/sunucu.sh" durdur drama >/dev/null 2>&1 || true
    continue
  fi
  D="$PROJE/Cikti/$K/sabit_drama3_duygu"
  [ -d "$D" ] || D="$PROJE/Cikti/$K/sabit_drama3"
  E="$(basename "$D")"
  # karşılaştırmak için WAV'ların kopyası kalsın (önizleme), paket için Opus
  find "$D" -name '*.wav' -print0 | xargs -0 -P "$(nproc)" -I{} sh -c 'ffmpeg -loglevel error -y -i "$1" -c:a libopus -b:a 128k "${1%.wav}.opus" && rm "$1"' _ {}
  ( cd "$PROJE" && rm -f "sonuc/${K}_v2.zip" && zip -qr "sonuc/${K}_v2.zip" "Cikti/$K/$E" "Cikti/$K/dokum_$E.json" "Cikti/$K/${K}_${E}_onizleme.wav" \
      "ForzaDublaj/calisma/$K/gunluk.txt" "ForzaDublaj/calisma/$K/ceviri_hymt2_30b.json" "ForzaDublaj/calisma/$K/yonler_omni.json" \
      "ForzaDublaj/calisma/$K/referans_drama.wav" "ForzaDublaj/calisma/$K/referans_drama.txt" )
  echo "$K bitti $(date +%H:%M:%S) ($(du -h "$PROJE/sonuc/${K}_v2.zip" | cut -f1))" | tee -a "$G"
done
echo "HEPSI BITTI $(date)" | tee -a "$G"
