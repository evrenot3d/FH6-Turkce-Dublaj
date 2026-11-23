#!/usr/bin/env bash
# Tek komut: kurulum + ilerleme sayfası + bütün seslerin dublajı (arka planda; SSH kapansa da sürer).
# Kullanım: bash ForzaDublaj/linux/baslat.sh [HTTP_PORT]   (port: Theta'da makineyi açarken girilen HTTP Port, varsayılan 8080)
set -euo pipefail
KOK="$(cd "$(dirname "$0")/.." && pwd)"
PROJE="$(dirname "$KOK")"
PORT="${1:-8080}"
bash "$KOK/linux/kur.sh"
# ilerleme sayfası: http://<sunucu>:<port>/ForzaDublaj/durum.html , sonuçlar: /sonuc/
nohup python3 -m http.server "$PORT" --bind 0.0.0.0 --directory "$PROJE" > "$KOK/http.log" 2>&1 &
nohup setsid bash "$KOK/linux/calistir.sh" > "$KOK/islem.log" 2>&1 &
echo
echo "Dublaj arka planda başladı."
echo "  İlerleme : http://<sunucu adresi>:$PORT/ForzaDublaj/durum.html"
echo "  Günlük   : tail -f $KOK/islem.log   ·   özet: cat $KOK/toplu_gunluk.txt"
echo "  Sonuçlar : $PROJE/sonuc/<karakter>.zip"
