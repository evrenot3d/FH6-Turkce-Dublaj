#!/usr/bin/env bash
# Tek komut: v2 kurulumu + sonuç indirme sayfası (port 8080) + dublaj (arka planda; SSH kapansa da sürer).
# Kullanım: bash ForzaDublaj/linux/v2/baslat_v2.sh Mei [Jordan ...]
set -euo pipefail
KOK="$(cd "$(dirname "$0")/../.." && pwd)"
PROJE="$(dirname "$KOK")"
bash "$KOK/linux/v2/kur_v2.sh"
pgrep -f "http.server 8080" >/dev/null || nohup python3 -m http.server 8080 --bind 0.0.0.0 --directory "$PROJE" > "$KOK/http.log" 2>&1 &
nohup setsid bash "$KOK/linux/v2/calistir_v2.sh" "$@" > "$KOK/islem_v2.log" 2>&1 &
echo
echo "Dublaj (v2) arka planda başladı."
echo "  Günlük   : tail -f $KOK/islem_v2.log"
echo "  Sonuçlar : $PROJE/sonuc/<karakter>_v2.zip  (port 8080'den /sonuc/)"
