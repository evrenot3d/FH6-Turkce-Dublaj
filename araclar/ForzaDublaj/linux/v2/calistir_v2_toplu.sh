#!/usr/bin/env bash
# v2 toplu dublaj: model sunucuları her karakterde yeniden açılmasın diye üç tur:
#   1. tur: bütün karakterler yazıya dökülür + çevrilir (Hy-MT2 30B sunucusu bir kez açılır)
#   2. tur: bütün karakterlerin oyunculuk yönleri (Qwen3-Omni bir kez açılır)
#   3. tur: seslendirme (Drama 3 bir kez açılır), her karakter bitince sonuc/<karakter>_v2.zip
# Sonucu (zip'i) olan karakter atlanır. Başka bir v2 işi (ör. Mei) sürüyorsa bitmesi beklenir.
# Kullanım: bash ForzaDublaj/linux/v2/calistir_v2_toplu.sh [Karakter ...]   (boşsa: 34 karakter + SatNav)
set -uo pipefail
KOK="$(cd "$(dirname "$0")/../.." && pwd)"
PROJE="$(dirname "$KOK")"
cd "$KOK"
. "$HOME/.cargo/env" 2>/dev/null || true
NV="$(python3 -c 'import glob,os,site;d=[p for s in site.getsitepackages() for p in glob.glob(os.path.join(s,"nvidia","*","lib"))];print(":".join(d))')"
export LD_LIBRARY_PATH="$KOK/bin/ort:$NV:${LD_LIBRARY_PATH:-}"
export FD_V2=1 FD_CEVIRI_MODEL=hymt2 FD_ACIK_KALSIN=1
mkdir -p "$PROJE/sonuc"
G="$KOK/toplu_gunluk_v2.txt"

while pgrep -f "calistir_v2\.sh" >/dev/null; do sleep 30; done

LISTE="Mei Jordan Announcer_Forzathon Yuji Keiko Helena Larry FoodJobHelper Isamu ScottTyler Robert Midori Announcer_Rocket BroadcastPresenter Mech ShowcasePilot1 ShowcasePilot2 Commentator1 FemaleNPC1 FemaleNPC2 FemaleNPC3 FemaleNPC4 MaleNPC1 MaleNPC2 MaleNPC3 MaleNPC4 RestaurantOwnerFemale1 RestaurantOwnerFemale2 RestaurantOwnerFemale3 RestaurantOwnerFemale4 RestaurantOwnerMale1 RestaurantOwnerMale2 RestaurantOwnerMale3 RestaurantOwnerMale4 SatNav"
[ $# -gt 0 ] && LISTE="$*"
KALAN=""
for K in $LISTE; do [ -f "$PROJE/sonuc/${K}_v2.zip" ] || KALAN="$KALAN $K"; done
echo "toplu v2 basladi $(date) ·$KALAN" | tee -a "$G"
calistir() { ./forzadublaj --toplu --karakter "$1" --dakika 0 --kip sabit --model drama3 "${@:2}"; }

for K in $KALAN; do
  echo "[1. tur] $K yazıya dökme + çeviri $(date +%H:%M:%S)" | tee -a "$G"
  calistir "$K" --sadece-ceviri || echo "$K 1. TUR HATA" | tee -a "$G"
done
for K in $KALAN; do
  echo "[2. tur] $K oyunculuk yönleri $(date +%H:%M:%S)" | tee -a "$G"
  calistir "$K" --sadece-yon || echo "$K 2. TUR HATA" | tee -a "$G"
done
for K in $KALAN; do
  echo "[3. tur] $K seslendirme $(date +%H:%M:%S)" | tee -a "$G"
  if ! calistir "$K"; then echo "$K 3. TUR HATA $(date +%H:%M:%S)" | tee -a "$G"; continue; fi
  D="$PROJE/Cikti/$K/sabit_drama3_duygu"
  [ -d "$D" ] || D="$PROJE/Cikti/$K/sabit_drama3"
  E="$(basename "$D")"
  find "$D" -name '*.wav' -print0 | xargs -0 -P "$(nproc)" -I{} sh -c 'ffmpeg -loglevel error -y -i "$1" -c:a libopus -b:a 128k "${1%.wav}.opus" && rm "$1"' _ {}
  ( cd "$PROJE" && rm -f "sonuc/${K}_v2.zip" && zip -qr "sonuc/${K}_v2.zip" "Cikti/$K/$E" "Cikti/$K/dokum_$E.json" "Cikti/$K/${K}_${E}_onizleme.wav" \
      "ForzaDublaj/calisma/$K/gunluk.txt" "ForzaDublaj/calisma/$K/ceviri_hymt2_30b.json" "ForzaDublaj/calisma/$K/yonler_omni.json" \
      "ForzaDublaj/calisma/$K/referans_drama.wav" "ForzaDublaj/calisma/$K/referans_drama.txt" )
  echo "$K bitti $(date +%H:%M:%S) ($(du -h "$PROJE/sonuc/${K}_v2.zip" | cut -f1))" | tee -a "$G"
done
FD_ACIK_KALSIN=0 bash "$KOK/linux/v2/sunucu.sh" durdur drama
echo "TOPLU V2 BITTI $(date)" | tee -a "$G"
