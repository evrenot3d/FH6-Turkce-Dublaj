#!/usr/bin/env bash
# Forza 6 Türkçe dublaj v2 — RunPod (L40S 48 GB / RTX PRO 6000) kurulumu.
# Kurar: sistem araçları, Rust, ONNX Runtime GPU (Whisper, konuşma algılama, kadın/erkek denetimi),
# vLLM + vLLM-Omni (ayrı sanal ortam: /root/v2env), modeller (Whisper, Hy-MT2 30B FP8, Qwen3-Omni AWQ, Drama 3),
# vgmstream; ForzaDublaj'ı derler. Çıktı: ForzaDublaj/kurulum_v2.log
set -euo pipefail
KOK="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$KOK"
exec > >(tee -a "$KOK/kurulum_v2.log") 2>&1
log() { echo "[kurulum $(date +%H:%M:%S)] $*"; }
export DEBIAN_FRONTEND=noninteractive PIP_BREAK_SYSTEM_PACKAGES=1
VENV="${FD_VENV:-/root/v2env}"
nvidia-smi --query-gpu=name,memory.total,driver_version --format=csv,noheader || true
df -h / | tail -1

log "1/7 sistem paketleri"
apt-get update -qq
apt-get install -y -qq git cmake build-essential curl ffmpeg unzip zip pkg-config libssl-dev ca-certificates >/dev/null

log "2/7 Rust"
command -v cargo >/dev/null || [ -x "$HOME/.cargo/bin/cargo" ] || curl -fsSL https://sh.rustup.rs | sh -s -- -y -q
. "$HOME/.cargo/env"

log "3/7 ONNX Runtime GPU (sistem Python'u)"
python3 -m pip install -q "onnxruntime-gpu[cuda,cudnn]>=1.22" huggingface_hub soundfile
ORT="$(python3 -c 'import onnxruntime,os;print(os.path.join(os.path.dirname(onnxruntime.__file__),"capi"))')"
mkdir -p bin/ort
for f in "$ORT"/libonnxruntime*.so*; do ln -sf "$f" "bin/ort/$(basename "$f")"; done
ln -sf "$(ls "$ORT"/libonnxruntime.so.* | head -1)" bin/ort/libonnxruntime.so

log "4/7 vLLM + vLLM-Omni ($VENV)"
command -v uv >/dev/null || { curl -LsSf https://astral.sh/uv/install.sh | sh; }
export PATH="$HOME/.local/bin:$PATH"
if [ ! -x "$VENV/bin/vllm" ]; then
  uv venv "$VENV" --python 3.12
  uv pip install --python "$VENV/bin/python" "vllm==0.33.0" --torch-backend=cu130
  uv pip install --python "$VENV/bin/python" "huggingface_hub[hf_xet]"
fi
# Drama 3: vLLM-Omni 0.33 (0.34 sürümünde Drama profili kaldırıldı: "V2-only execution"), ayrı ortamda
DVENV="${FD_DRAMA_VENV:-/root/v2drama}"
if [ ! -x "$DVENV/bin/vllm" ]; then
  uv venv "$DVENV" --python 3.12
  uv pip install --python "$DVENV/bin/python" "vllm==0.33.0" --torch-backend=cu130
  uv pip install --python "$DVENV/bin/python" "git+https://github.com/vllm-project/vllm-omni.git@v0.33.0rc1"
fi
"$DVENV/bin/python" -c "import vllm, vllm_omni; print('Drama ortamı: vllm', vllm.__version__)"
# 48 GB kart (L40S) için Drama 3 dağıtım ayarı: CUDA grafikleri kapalı, kısa bağlam, 8 eşzamanlı istek
"$DVENV/bin/python" - "$KOK/linux/v2/drama_48gb.yaml" <<'PY'
import sys, yaml, vllm_omni, os
src = os.path.join(os.path.dirname(vllm_omni.__file__), "deploy", "fish_omni.yaml")
d = yaml.safe_load(open(src))
def gez(o):
    if isinstance(o, dict):
        if "stage_id" in o:
            yield o
        for v in o.values():
            yield from gez(v)
    elif isinstance(o, list):
        for v in o:
            yield from gez(v)
for s in gez(d):
    if s["stage_id"] == 0:
        s.update(gpu_memory_utilization=0.885, max_model_len=2048, max_num_batched_tokens=2048, max_num_seqs=8, enforce_eager=True)
        s.pop("compilation_config", None)
    else:
        s.update(gpu_memory_utilization=0.06, max_model_len=16384, max_num_batched_tokens=16384, max_num_seqs=8)
yaml.safe_dump(d, open(sys.argv[1], "w"), sort_keys=False)
print("drama_48gb.yaml yazıldı")
PY
"$VENV/bin/python" -c "import torch, vllm; print('torch', torch.__version__, 'cuda', torch.cuda.is_available(), torch.cuda.get_device_name(0), '| vllm', vllm.__version__)"

log "5/7 modeller (Whisper, Hy-MT2 30B FP8, Qwen3-Omni AWQ, Drama 3) — toplam ~90 GB"
"$VENV/bin/python" - <<'PY'
from huggingface_hub import snapshot_download
import sys
snapshot_download("onnx-community/whisper-large-v3-turbo_timestamped", local_dir="modeller/whisper-onnx",
                  allow_patterns=["*.json", "onnx/encoder_model_fp16.onnx*", "onnx/decoder_model_merged_fp16.onnx*"])
for r in ["tencent/Hy-MT2-30B-A3B-FP8", "cpatonn/Qwen3-Omni-30B-A3B-Instruct-AWQ-4bit", "fishaudio/drama-3"]:
    print("indiriliyor:", r, flush=True)
    try:
        snapshot_download(r)
    except Exception as e:
        print(f"\n!!! {r} indirilemedi: {e}\n!!! Model izin istiyorsa: huggingface.co/{r} sayfasında lisansı kabul edip"
              " 'hf auth login' ile giriş yapın, sonra kurulumu yeniden çalıştırın.", flush=True)
        sys.exit(1)
print("modeller tamam")
PY

log "6/7 vgmstream"
if [ ! -x vgmstream/vgmstream-cli ]; then
  mkdir -p vgmstream && cd vgmstream
  curl -fL -o p.tgz https://github.com/vgmstream/vgmstream-releases/releases/download/nightly/vgmstream-linux-cli.tar.gz
  tar xzf p.tgz && rm p.tgz
  B="$(find . -name vgmstream-cli -type f | head -1)"; [ "$B" != "./vgmstream-cli" ] && cp "$B" ./vgmstream-cli
  chmod +x vgmstream-cli
  cd "$KOK"
fi

log "7/7 ForzaDublaj derleniyor"
# yollar.rs: kendi modeller/ ve bin/ klasörleri kullanılsın (yoksa Windows'taki Evrenot yolu aranır)
mkdir -p modeller/zonos3 bin/zonos3
cargo build --release -q
cp target/release/forzadublaj ./forzadublaj
log "KURULUM TAMAM"
