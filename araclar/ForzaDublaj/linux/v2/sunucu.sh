#!/usr/bin/env bash
# v2 model sunucuları (vLLM). ForzaDublaj programı çağırır:
#   sunucu.sh baslat ceviri|omni|drama   → açar, hazır olana kadar bekler (zaten açıksa bir şey yapmaz)
#   sunucu.sh durdur ceviri|omni|drama   → süreç grubunu tamamen kapatır, ekran kartı boşalana kadar bekler
#   sunucu.sh yon <görev.json> <çıkış.json> → Qwen3-Omni ile cümle cümle oyunculuk yönleri
# Her sunucu kendi süreç grubunda açılır (setsid); vLLM'in alt süreçleri de birlikte kapanır.
set -uo pipefail
# calistir_v2.sh ONNX Runtime için pip'in nvidia (CUDA 12) kütüphanelerini LD_LIBRARY_PATH'e koyar; vLLM (torch cu130)
# onlarla açılırsa cuDNN yüklenemez (CUDNN_STATUS_SUBLIBRARY_LOADING_FAILED) → sunucular temiz yolla açılır
unset LD_LIBRARY_PATH
KOK="$(cd "$(dirname "$0")/../.." && pwd)"
VENV="${FD_VENV:-/root/v2env}"
# shellcheck disable=SC1091
. "$VENV/bin/activate"
EYLEM="${1:-}"
AD="${2:-}"

port() {
  case "$1" in
    ceviri) echo 17893 ;;
    drama) echo 17896 ;;
    omni) echo 17897 ;;
    *) echo "bilinmeyen sunucu: $1" >&2; exit 2 ;;
  esac
}

komut_kur() {
  # KOMUT dizisini doldurur (tırnaklı JSON argümanı bozulmasın diye eval kullanılmaz)
  case "$1" in
    ceviri) KOMUT=(vllm serve tencent/Hy-MT2-30B-A3B-FP8 --served-model-name hymt2 --host 127.0.0.1 --port 17893
                   --max-model-len 2048 --gpu-memory-utilization 0.80 --max-num-seqs 64 --trust-remote-code) ;;
    omni) KOMUT=(vllm serve cpatonn/Qwen3-Omni-30B-A3B-Instruct-AWQ-4bit --served-model-name omni --host 127.0.0.1 --port 17897
                 --max-model-len 4096 --gpu-memory-utilization 0.90 --max-num-seqs 16 --trust-remote-code
                 --limit-mm-per-prompt '{"audio":1,"image":0,"video":0}') ;;
    drama) KOMUT=("${FD_DRAMA_VENV:-/root/v2drama}/bin/vllm" serve fishaudio/drama-3 --omni --deploy-config "$KOK/linux/v2/drama_48gb.yaml" --host 127.0.0.1 --port 17896) ;;
  esac
}

saglikli() { curl -sf -m 3 "http://127.0.0.1:$(port "$1")/health" >/dev/null 2>&1; }

gpu_kullanilan() { nvidia-smi --query-gpu=memory.used --format=csv,noheader,nounits | head -1 | tr -d ' '; }

durdur() {
  local ad="$1" pidf="$KOK/.sunucu_$1.pid"
  if [ -f "$pidf" ]; then
    local pg; pg="$(cat "$pidf")"
    echo "[sunucu] $ad kapatılıyor (grup $pg)"
    kill -TERM -- "-$pg" 2>/dev/null || true
    for _ in $(seq 1 60); do kill -0 -- "-$pg" 2>/dev/null || break; sleep 1; done
    kill -KILL -- "-$pg" 2>/dev/null || true
    rm -f "$pidf"
  fi
  # ekran kartı boşalsın (sonraki model yer bulsun)
  for _ in $(seq 1 90); do
    [ "$(gpu_kullanilan)" -lt 2500 ] && break
    sleep 1
  done
  echo "[sunucu] $ad kapalı · ekran kartında $(gpu_kullanilan) MB dolu"
}

baslat() {
  local ad="$1" pidf="$KOK/.sunucu_$1.pid" log="$KOK/sunucu_$1.log"
  if saglikli "$ad"; then echo "[sunucu] $ad zaten açık"; return 0; fi
  # başka bir model sunucusu açık kalmışsa kapat (kart tek modele yetiyor)
  for d in ceviri omni drama; do [ "$d" != "$ad" ] && [ -f "$KOK/.sunucu_$d.pid" ] && durdur "$d"; done
  komut_kur "$ad"
  echo "[sunucu] $ad açılıyor: ${KOMUT[*]}"
  setsid nohup "${KOMUT[@]}" > "$log" 2>&1 < /dev/null &
  echo $! > "$pidf"
  local t0=$SECONDS
  while [ $((SECONDS - t0)) -lt 3000 ]; do
    if saglikli "$ad"; then echo "[sunucu] $ad hazır ($((SECONDS - t0)) sn)"; return 0; fi
    if ! kill -0 "$(cat "$pidf")" 2>/dev/null; then
      echo "[sunucu] $ad KAPANDI. Son satırlar ($log):"; tail -n 40 "$log"; rm -f "$pidf"; return 1
    fi
    sleep 5
  done
  echo "[sunucu] $ad zamanında açılmadı ($log)"; tail -n 40 "$log"; durdur "$ad"; return 1
}

case "$EYLEM" in
  baslat) baslat "$AD" ;;
  durdur)
    # toplu çalışmada (FD_ACIK_KALSIN=1) sunucu açık kalır; başka model açılırken baslat onu zaten kapatır
    if [ "${FD_ACIK_KALSIN:-0}" = 1 ]; then echo "[sunucu] $AD açık kalıyor"; else durdur "$AD"; fi ;;
  yon) python "$KOK/linux/v2/yon_cikar.py" --gorev "$2" --cikis "$3" --port 17897 ;;
  *) echo "kullanım: $0 baslat|durdur ceviri|omni|drama  |  $0 yon <görev> <çıkış>"; exit 2 ;;
esac
