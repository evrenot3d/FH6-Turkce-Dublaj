#!/usr/bin/env python3
"""VoxCPM2 sunucusu (bulut / Linux). Windows'taki audio.cpp sunucusunun ForzaDublaj'ın kullandığı kısmının aynısı:
  GET  /health          → model yüklendiyse 200
  POST /v1/tasks/run    {"model": "vox", "request": {"text", "language", "seed", "voice_ref", "options": {"num_inference_steps"}}}
                        → {"audio": "<base64 WAV>"}
Model: resmî openbmb/VoxCPM2 (PyTorch, tam hassasiyet). Ses rengi voice_ref'ten (metinsiz klonlama), tohum sabit.
"""
import argparse
import base64
import inspect
import io
import json
import random
import sys
import traceback
from http.server import BaseHTTPRequestHandler, HTTPServer

import numpy as np
import soundfile as sf
import torch
from voxcpm import VoxCPM

ap = argparse.ArgumentParser()
ap.add_argument("--port", type=int, default=17895)
ap.add_argument("--model", default="openbmb/VoxCPM2")
args = ap.parse_args()

print(f"[vox] {args.model} yükleniyor…", flush=True)
MODEL = VoxCPM.from_pretrained(args.model, load_denoiser=False)
SR = MODEL.tts_model.sample_rate
PARAM = set(inspect.signature(MODEL.generate).parameters)
print(f"[vox] hazır, {SR} Hz; generate parametreleri: {sorted(PARAM)}", flush=True)


def uret(istek: dict) -> bytes:
    metin = istek["text"]
    tohum = int(istek.get("seed", 1234)) % 4294967295
    adim = int((istek.get("options") or {}).get("num_inference_steps", 10))
    ref = istek.get("voice_ref")
    random.seed(tohum)
    np.random.seed(tohum % (2**32 - 1))
    torch.manual_seed(tohum)
    torch.cuda.manual_seed_all(tohum)
    kw = {"text": metin, "cfg_value": 2.0, "inference_timesteps": adim}
    if ref:
        kw["reference_wav_path"] = ref
    if "seed" in PARAM:
        kw["seed"] = tohum
    with torch.inference_mode():
        wav = MODEL.generate(**kw)
    wav = np.asarray(wav, dtype=np.float32).reshape(-1)
    b = io.BytesIO()
    sf.write(b, wav, SR, format="WAV", subtype="PCM_16")
    return b.getvalue()


class Isleyici(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def _yanit(self, kod, govde):
        b = json.dumps(govde).encode()
        self.send_response(kod)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(b)))
        self.end_headers()
        self.wfile.write(b)

    def do_GET(self):
        if self.path.startswith("/health"):
            self._yanit(200, {"status": "ok"})
        else:
            self._yanit(404, {"error": "yok"})

    def do_POST(self):
        try:
            n = int(self.headers.get("Content-Length", 0))
            g = json.loads(self.rfile.read(n))
            wav = uret(g.get("request", g))
            self._yanit(200, {"audio": base64.b64encode(wav).decode()})
        except Exception as e:
            traceback.print_exc(file=sys.stderr)
            self._yanit(500, {"error": f"{type(e).__name__}: {e}"})


HTTPServer(("127.0.0.1", args.port), Isleyici).serve_forever()
