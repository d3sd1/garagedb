"""Pasada 2 del piloto OCR: mosaico con solape + upscale 2x — el preprocesado
que la etapa de ingesta aplica de serie. Mismo motor (PP-OCRv4 ONNX CPU).
Dedupe por texto+centro. Compara full-frame vs tiled y regenera contact sheet."""
import json
import re
import time
from pathlib import Path

import numpy as np
from PIL import Image
from rapidocr_onnxruntime import RapidOCR

SRC = Path(r"D:\clarividence\datasets\garage-site\2026-08-10")
HERE = Path(__file__).parent
TILE = 640
OVERLAP = 160
UPSCALE = 2

MATRIX = re.compile(r"^[A-Z]\d{1,2}[-_]?[A-Z]?\d{1,2}[A-Z]?\d{0,2}$")

engine = RapidOCR()

all_rows = []
per_image = []
for img_path in sorted(SRC.glob("site-*.jpg")):
    img = Image.open(img_path)
    W, H = img.size
    t0 = time.perf_counter()
    dets = []
    step = TILE - OVERLAP
    for y in range(0, H, step):
        for x in range(0, W, step):
            tile = img.crop((x, y, min(x + TILE, W), min(y + TILE, H)))
            if tile.size[0] < 60 or tile.size[1] < 60:
                continue
            tile_up = tile.resize((tile.size[0] * UPSCALE, tile.size[1] * UPSCALE))
            result, _ = engine(np.array(tile_up))
            for box, text, conf in result or []:
                clean = text.strip().upper().replace(" ", "")
                if MATRIX.match(clean) and any(c.isdigit() for c in clean):
                    xs = [p[0] / UPSCALE + x for p in box]
                    ys = [p[1] / UPSCALE + y for p in box]
                    dets.append({
                        "text": clean,
                        "conf": round(float(conf), 3),
                        "cx": sum(xs) / 4,
                        "cy": sum(ys) / 4,
                        "bbox": [int(min(xs)), int(min(ys)), int(max(xs)), int(max(ys))],
                    })
    dt = time.perf_counter() - t0
    # dedupe: mismo texto con centros a <40 px = misma etiqueta (solape de tiles)
    dets.sort(key=lambda d: -d["conf"])
    kept = []
    for d in dets:
        if not any(k["text"] == d["text"] and abs(k["cx"] - d["cx"]) < 40 and abs(k["cy"] - d["cy"]) < 40 for k in kept):
            kept.append(d)
    for d in kept:
        all_rows.append({"image": img_path.name, **{k: d[k] for k in ("text", "conf", "bbox")}})
    per_image.append({"image": img_path.name, "matrix_labels": len(kept), "latency_s": round(dt, 1)})
    print(f"{img_path.name}: {len(kept)} etiquetas matriciales, {dt:.0f}s")

summary = {
    "engine": "RapidOCR (PP-OCRv4 ONNX, CPU), tiled 640px overlap 160 upscale 2x",
    "n_images": len(per_image),
    "matrix_labels_total": len(all_rows),
    "matrix_labels_unique_text": len({r["text"] for r in all_rows}),
    "mean_conf": round(float(np.mean([r["conf"] for r in all_rows])), 3),
    "mean_latency_s": round(float(np.mean([p["latency_s"] for p in per_image])), 1),
    "per_image": per_image,
}
(HERE / "results" / "ocr_pilot_tiled.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")

import csv
with open(HERE / "results" / "ocr_pilot_labels.csv", "w", newline="", encoding="utf-8") as f:
    w = csv.DictWriter(f, fieldnames=["image", "text", "conf", "bbox"])
    w.writeheader()
    w.writerows(all_rows)

# contact sheet
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt

rows_sorted = sorted(all_rows, key=lambda r: (r["image"], r["text"]))
n = len(rows_sorted)
cols = 10
rows_n = max(1, (n + cols - 1) // cols)
fig, axes = plt.subplots(rows_n, cols, figsize=(cols * 1.4, rows_n * 1.0))
axes = np.atleast_2d(axes)
imgs = {}
for i, r in enumerate(rows_sorted):
    ax = axes[i // cols][i % cols]
    if r["image"] not in imgs:
        imgs[r["image"]] = Image.open(SRC / r["image"])
    x0, y0, x1, y1 = r["bbox"]
    pad = 8
    crop = imgs[r["image"]].crop((max(0, x0 - pad), max(0, y0 - pad), x1 + pad, y1 + pad))
    ax.imshow(crop)
    ax.set_title(f"{r['text']} ({r['conf']:.2f})", fontsize=4.5)
for ax in axes.flat:
    ax.axis("off")
fig.tight_layout()
fig.savefig(HERE / "figures" / "ocr_contact_sheet.pdf", dpi=150)
print(json.dumps({k: v for k, v in summary.items() if k != "per_image"}, indent=2))
