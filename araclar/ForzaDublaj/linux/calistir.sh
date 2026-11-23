#!/usr/bin/env bash
# Bütün sesler: 34 karakter → navigasyon → 9 radyo DJ'i. VoxCPM2 30 adım, duygusuz, sabit referans + sabit tohum.
# Her karakter bitince Türkçe sesleri Opus'a sıkıştırıp sonuc/<karakter>.zip yapar (rclone ayarlıysa Drive'a da yükler).
# İlerleme: http://<sunucu>:<port>/ForzaDublaj/durum.html · toplu günlük: ForzaDublaj/toplu_gunluk.txt
set -uo pipefail
KOK="$(cd "$(dirname "$0")/.." && pwd)"
PROJE="$(dirname "$KOK")"
cd "$KOK"
. "$HOME/.cargo/env" 2>/dev/null || true
# ONNX Runtime'ın CUDA / cuDNN kütüphaneleri (pip'teki nvidia paketleri)
NV="$(python3 -c 'import glob,os,site;d=[p for s in site.getsitepackages() for p in glob.glob(os.path.join(s,"nvidia","*","lib"))];print(":".join(d))')"
export LD_LIBRARY_PATH="$KOK/bin/ort:$NV:${LD_LIBRARY_PATH:-}"
mkdir -p "$PROJE/sonuc"
G="$KOK/toplu_gunluk.txt"
echo "basladi $(date)" >> "$G"

KARAKTERLER="Mei Jordan Announcer_Forzathon Yuji Keiko Helena Larry FoodJobHelper Isamu ScottTyler Robert Midori Announcer_Rocket BroadcastPresenter Mech ShowcasePilot1 ShowcasePilot2 Commentator1 FemaleNPC1 FemaleNPC2 FemaleNPC3 FemaleNPC4 MaleNPC1 MaleNPC2 MaleNPC3 MaleNPC4 RestaurantOwnerFemale1 RestaurantOwnerFemale2 RestaurantOwnerFemale3 RestaurantOwnerFemale4 RestaurantOwnerMale1 RestaurantOwnerMale2 RestaurantOwnerMale3 RestaurantOwnerMale4 SatNav DJ_01 DJ_02 DJ_03 DJ_04 DJ_05 DJ_06 DJ_07 DJ_08 DJ_09"
[ -n "${1:-}" ] && KARAKTERLER="$*"

for K in $KARAKTERLER; do
  if [ -f "$PROJE/sonuc/$K.zip" ]; then echo "$K zaten bitmiş, atlandı" >> "$G"; continue; fi
  echo "$K basladi $(date +%H:%M:%S)" >> "$G"
  if ! ./forzadublaj --toplu --karakter "$K" --dakika 0 --kip sabit --model voxcpm2 --adim 30 --duygusuz; then
    echo "$K HATA $(date +%H:%M:%S) (calisma/$K/gunluk.txt)" >> "$G"
    continue
  fi
  D="$PROJE/Cikti/$K/sabit_voxcpm2_30"
  # WAV → Opus (128 kb/sn), çekirdek sayısı kadar paralel
  find "$D" -name '*.wav' -print0 | xargs -0 -P "$(nproc)" -I{} sh -c 'ffmpeg -loglevel error -y -i "$1" -c:a libopus -b:a 128k "${1%.wav}.opus" && rm "$1"' _ {}
  rm -f "$PROJE/Cikti/$K/"*_onizleme.wav
  rm -rf "$PROJE/Cikti/$K/ingilizce"
  ( cd "$PROJE" && zip -qr "sonuc/$K.zip" "Cikti/$K/sabit_voxcpm2_30" "Cikti/$K/dokum_sabit_voxcpm2_30.json" "ForzaDublaj/calisma/$K/gunluk.txt" "ForzaDublaj/calisma/$K/ceviri.json" "ForzaDublaj/calisma/$K/referans.wav" )
  if command -v rclone >/dev/null && rclone listremotes 2>/dev/null | grep -q '^gdrive:'; then
    rclone copy "$PROJE/sonuc/$K.zip" "gdrive:Forza6_Turkce_Dublaj" && echo "$K Drive'a yuklendi" >> "$G"
  fi
  echo "$K bitti $(date +%H:%M:%S) ($(du -h "$PROJE/sonuc/$K.zip" | cut -f1))" >> "$G"
done
echo "HEPSI BITTI $(date)" >> "$G"
