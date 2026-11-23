#!/usr/bin/env bash
# Forza 6 Türkçe dublaj — bulut sunucusu kurulumu (RTX 5090; Ubuntu + CUDA + PyTorch imajı).
# Kurar: sistem araçları, CUDA derleyicisi (yoksa), Rust, Python paketleri (VoxCPM2, ONNX Runtime GPU),
# modeller (Whisper, Hy-MT2 Q8_0, VoxCPM2), llama.cpp (CUDA), vgmstream; ForzaDublaj'ı derler.
# Çıktı: ForzaDublaj/kurulum_linux.log
set -euo pipefail
KOK="$(cd "$(dirname "$0")/.." && pwd)"
cd "$KOK"
exec > >(tee -a "$KOK/kurulum_linux.log") 2>&1
log() { echo "[kurulum $(date +%H:%M:%S)] $*"; }
export DEBIAN_FRONTEND=noninteractive
SUDO=""; [ "$(id -u)" != 0 ] && SUDO="sudo"
nvidia-smi --query-gpu=name,memory.total,driver_version --format=csv,noheader || true

log "1/8 sistem paketleri"
$SUDO apt-get update -qq
$SUDO apt-get install -y -qq git cmake build-essential curl ffmpeg unzip zip pkg-config libssl-dev ca-certificates >/dev/null

log "2/8 CUDA derleyicisi"
for d in /usr/local/cuda/bin /usr/local/cuda-*/bin; do [ -x "$d/nvcc" ] && export PATH="$d:$PATH" && break; done
if ! command -v nvcc >/dev/null; then
  . /etc/os-release
  V="${VERSION_ID//./}"
  curl -fsSL -o /tmp/cuda-keyring.deb "https://developer.download.nvidia.com/compute/cuda/repos/ubuntu${V}/x86_64/cuda-keyring_1.1-1_all.deb"
  $SUDO dpkg -i /tmp/cuda-keyring.deb >/dev/null
  $SUDO apt-get update -qq
  $SUDO apt-get install -y -qq cuda-nvcc-12-8 cuda-cudart-dev-12-8 libcublas-dev-12-8 >/dev/null
  export PATH="/usr/local/cuda-12.8/bin:$PATH"
fi
nvcc --version | tail -1

log "3/8 Rust"
command -v cargo >/dev/null || [ -x "$HOME/.cargo/bin/cargo" ] || curl -fsSL https://sh.rustup.rs | sh -s -- -y -q
. "$HOME/.cargo/env"

log "4/8 Python paketleri"
python3 -m pip install -q -U pip
python3 -m pip install -q "huggingface_hub[cli]" gdown soundfile voxcpm "onnxruntime-gpu[cuda,cudnn]>=1.22"
python3 -c "import torch; print('torch', torch.__version__, 'cuda', torch.cuda.is_available(), torch.cuda.get_device_name(0))"
# ONNX Runtime (Whisper, konuşma algılama, kadın/erkek denetimi): pip paketinin kütüphaneleri bin/ort'a bağlanır
ORT="$(python3 -c 'import onnxruntime,os;print(os.path.join(os.path.dirname(onnxruntime.__file__),"capi"))')"
mkdir -p bin/ort
for f in "$ORT"/libonnxruntime*.so*; do ln -sf "$f" "bin/ort/$(basename "$f")"; done
ln -sf "$(ls "$ORT"/libonnxruntime.so.* | head -1)" bin/ort/libonnxruntime.so

log "5/8 modeller"
python3 - <<'PY'
from huggingface_hub import snapshot_download, hf_hub_download
import os, shutil
snapshot_download("onnx-community/whisper-large-v3-turbo_timestamped", local_dir="modeller/whisper-onnx",
                  allow_patterns=["*.json", "onnx/encoder_model_fp16.onnx*", "onnx/decoder_model_merged_fp16.onnx*"])
os.makedirs("modeller/hy-mt2", exist_ok=True)
if not os.path.isfile("modeller/hy-mt2/Hy-MT2-7B-Q8_0.gguf"):
    p = hf_hub_download("tencent/Hy-MT2-7B-GGUF", "HY-MT2-7B-Q8_0.gguf", local_dir="modeller/hy-mt2")
    shutil.move(p, "modeller/hy-mt2/Hy-MT2-7B-Q8_0.gguf")
from voxcpm import VoxCPM
VoxCPM.from_pretrained("openbmb/VoxCPM2", load_denoiser=False)
print("modeller tamam")
PY

log "6/8 llama.cpp (CUDA, RTX 50)"
if [ ! -x bin/llama-cuda/llama-server ]; then
  rm -rf /tmp/llama.cpp
  git clone -q --depth 1 https://github.com/ggml-org/llama.cpp /tmp/llama.cpp
  cmake -S /tmp/llama.cpp -B /tmp/llama.cpp/build -DGGML_CUDA=ON -DCMAKE_CUDA_ARCHITECTURES=120 -DLLAMA_CURL=OFF \
        -DBUILD_SHARED_LIBS=OFF -DCMAKE_BUILD_TYPE=Release >/dev/null
  cmake --build /tmp/llama.cpp/build -j"$(nproc)" --target llama-server >/dev/null
  mkdir -p bin/llama-cuda
  cp /tmp/llama.cpp/build/bin/llama-server bin/llama-cuda/
fi

log "7/8 vgmstream"
if [ ! -x vgmstream/vgmstream-cli ]; then
  URL="$(curl -fsSL https://api.github.com/repos/vgmstream/vgmstream/releases/latest | python3 -c 'import sys,json;a=[x["browser_download_url"] for x in json.load(sys.stdin)["assets"] if "linux" in x["name"].lower() and "cli" in x["name"].lower()];print(a[0])')"
  mkdir -p vgmstream && cd vgmstream
  curl -fsSL -o paket "$URL"
  case "$URL" in *.zip) unzip -oq paket ;; *) tar xf paket ;; esac
  rm -f paket
  B="$(find . -name vgmstream-cli -type f | head -1)"; [ "$B" != "./vgmstream-cli" ] && cp "$B" ./vgmstream-cli
  chmod +x vgmstream-cli
  cd "$KOK"
fi
./vgmstream/vgmstream-cli -V 2>/dev/null | head -1 || true

log "8/8 ForzaDublaj derleniyor"
cargo build --release -q
cp target/release/forzadublaj ./forzadublaj
log "KURULUM TAMAM"
