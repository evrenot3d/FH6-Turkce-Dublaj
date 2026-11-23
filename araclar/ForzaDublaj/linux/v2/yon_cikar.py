#!/usr/bin/env python3
"""Cümle cümle oyunculuk yönü: Qwen3-Omni (vLLM sunucusu, port 17897) İngilizce sesi dinler, Drama 3'e verilecek
kısa bir İngilizce yön yazar ("cheerful, upbeat", "sarcastic", "whispering, nervous") ya da "neutral".

Girdi  (--gorev): [{"anahtar": "12:3.45", "wav": ".../yon/0012_000345.wav", "en": "English sentence"}, ...]
Çıkış (--cikis): {"12:3.45": "cheerful, upbeat", ...}  (varsa üzerine eklenir; her 50 cümlede bir kaydedilir)
"""
import argparse
import base64
import json
import os
import sys
import threading
import time
import urllib.request
from concurrent.futures import ThreadPoolExecutor, as_completed

ISTEM = (
    "You are a voice director for a video game dub. Listen to how this line is performed by the actor.\n"
    "Transcript: \"{en}\"\n"
    "Describe HOW it is delivered (emotion, energy, attitude) as a short English acting direction of 1 to 5 words, "
    "for example: cheerful, upbeat / calm and warm / sarcastic / excited, fast / whispering, nervous / annoyed / "
    "proud, confident / sad, quiet. If the delivery is plain, ordinary and emotionally neutral, answer exactly: neutral.\n"
    "Output only the direction, nothing else."
)


def sor(port, wav, en, deneme=3):
    veri = base64.b64encode(open(wav, "rb").read()).decode("ascii")
    govde = {
        "model": "omni",
        "messages": [{
            "role": "user",
            "content": [
                {"type": "audio_url", "audio_url": {"url": f"data:audio/wav;base64,{veri}"}},
                {"type": "text", "text": ISTEM.format(en=en.replace('"', "'"))},
            ],
        }],
        "temperature": 0.0,
        "max_tokens": 20,
    }
    son = None
    for k in range(deneme):
        try:
            r = urllib.request.Request(
                f"http://127.0.0.1:{port}/v1/chat/completions",
                data=json.dumps(govde).encode(),
                headers={"Content-Type": "application/json"},
            )
            with urllib.request.urlopen(r, timeout=300) as y:
                v = json.load(y)
            t = (v["choices"][0]["message"]["content"] or "").strip()
            t = t.split("\n")[0].strip().strip(".\"'[]").strip()
            if len(t) > 60:
                t = t[:60].rsplit(" ", 1)[0]
            return t.lower() or "neutral"
        except Exception as e:  # noqa: BLE001
            son = e
            time.sleep(1 + 2 * k)
    raise RuntimeError(f"{os.path.basename(wav)}: {son}")


def main():
    a = argparse.ArgumentParser()
    a.add_argument("--gorev", required=True)
    a.add_argument("--cikis", required=True)
    a.add_argument("--port", type=int, default=17897)
    a.add_argument("--es", type=int, default=16, help="aynı anda istek")
    o = a.parse_args()

    gorev = json.load(open(o.gorev, encoding="utf-8"))
    sonuc = {}
    if os.path.isfile(o.cikis):
        try:
            sonuc = json.load(open(o.cikis, encoding="utf-8"))
        except Exception:  # noqa: BLE001
            sonuc = {}
    kalan = [g for g in gorev if g["anahtar"] not in sonuc]
    print(f"[yön] {len(kalan)} cümle Qwen3-Omni'ye soruluyor ({len(gorev) - len(kalan)} önbellekte)", flush=True)
    kilit = threading.Lock()

    def kaydet():
        gecici = o.cikis + ".tmp"
        with open(gecici, "w", encoding="utf-8") as f:
            json.dump(sonuc, f, ensure_ascii=False, indent=1)
        os.replace(gecici, o.cikis)

    hata = 0
    t0 = time.time()
    with ThreadPoolExecutor(max_workers=o.es) as h:
        isler = {h.submit(sor, o.port, g["wav"], g["en"]): g for g in kalan}
        for n, f in enumerate(as_completed(isler), 1):
            g = isler[f]
            try:
                yon = f.result()
            except Exception as e:  # noqa: BLE001
                hata += 1
                print(f"[yön] hata: {e}", file=sys.stderr, flush=True)
                continue
            with kilit:
                sonuc[g["anahtar"]] = yon
                if n % 50 == 0:
                    kaydet()
            if n % 25 == 0 or n == len(kalan):
                print(f"[yön] {n}/{len(kalan)} · {time.time() - t0:.0f} sn · {g['en'][:50]!r} → {yon}", flush=True)
    kaydet()
    print(f"[yön] bitti: {len(sonuc)} cümle, {hata} hata", flush=True)
    if hata > len(kalan) // 2 and kalan:
        sys.exit(1)


if __name__ == "__main__":
    main()
