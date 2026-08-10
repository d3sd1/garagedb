"""Piloto REAL de la etapa OCR de la cascada sobre el etiquetado matricial
preexistente del taller (12 fotos de sitio, 2026-08-10).

Motor: RapidOCR (PP-OCR v4 en ONNX Runtime, CPU) — el modelo declarado en la
Tabla 2 del paper, en su empaquetado ONNX.

Salidas:
  results/ocr_pilot.json      — detecciones, latencias, conteos
  results/ocr_pilot_labels.csv — (foto, texto, conf) de candidatos a etiqueta matricial
  figures/ocr_contact_sheet.pdf — crops de candidatos para verificación humana
"""
import json
import re
import time
from pathlib import Path

import numpy as np
from PIL import Image
from rapidocr_onnxruntime import RapidOCR

SRC = Path(r"D:\clarividence\datasets\garage-site\2026-08-10")
HERE = Path(__file__).parent

# etiqueta matricial: D2-F07C2, D3-F04C1, A1-F1C3, B3_L5, T2-D07...
MATRIX = re.compile(r"^[A-Z]\d{1,2}[-_]?[A-Z]?\d{1,2}[A-Z]?\d{0,2}$")

engine = RapidOCR()

all_rows = []
per_image = []
for img_path in sorted(SRC.glob("site-*.jpg")):
    t0 = time.perf_counter()
    result, _ = engine(str(img_path))
    dt = time.perf_counter() - t0
    result = result or []
    n_matrix = 0
    for box, text, conf in result:
        clean = text.strip().upper().replace(" ", "")
        is_matrix = bool(MATRIX.match(clean)) and any(c.isdigit() for c in clean)
        if is_matrix:
            n_matrix += 1
            xs = [p[0] for p in box]
            ys = [p[1] for p in box]
            all_rows.append({
                "image": img_path.name,
                "text": clean,
                "conf": round(float(conf), 3),
                "bbox": [int(min(xs)), int(min(ys)), int(max(xs)), int(max(ys))],
            })
    per_image.append({
        "image": img_path.name,
        "detections_total": len(result),
        "matrix_candidates": n_matrix,
        "latency_s": round(dt, 2),
    })
    print(f"{img_path.name}: {len(result)} det, {n_matrix} matrix, {dt:.1f}s")

summary = {
    "engine": "RapidOCR (PP-OCRv4 ONNX, CPU)",
    "n_images": len(per_image),
    "total_detections": sum(p["detections_total"] for p in per_image),
    "total_matrix_candidates": len(all_rows),
    "unique_matrix_labels": len({r["text"] for r in all_rows}),
    "mean_conf_matrix": round(float(np.mean([r["conf"] for r in all_rows])), 3) if all_rows else None,
    "mean_latency_s": round(float(np.mean([p["latency_s"] for p in per_image])), 2),
    "per_image": per_image,
}
(HERE / "results" / "ocr_pilot.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")

import csv
with open(HERE / "results" / "ocr_pilot_labels.csv", "w", newline="", encoding="utf-8") as f:
    w = csv.DictWriter(f, fieldnames=["image", "text", "conf", "bbox"])
    w.writeheader()
    w.writerows(all_rows)

# contact sheet de crops para verificación humana
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt

rows_sorted = sorted(all_rows, key=lambda r: -r["conf"])
n = len(rows_sorted)
cols = 8
rows_n = max(1, (n + cols - 1) // cols)
fig, axes = plt.subplots(rows_n, cols, figsize=(cols * 1.5, rows_n * 1.1))
axes = np.atleast_2d(axes)
imgs = {}
for i, r in enumerate(rows_sorted):
    ax = axes[i // cols][i % cols]
    if r["image"] not in imgs:
        imgs[r["image"]] = Image.open(SRC / r["image"])
    x0, y0, x1, y1 = r["bbox"]
    pad = 6
    crop = imgs[r["image"]].crop((x0 - pad, y0 - pad, x1 + pad, y1 + pad))
    ax.imshow(crop)
    ax.set_title(f"{r['text']} ({r['conf']:.2f})", fontsize=5)
for ax in axes.flat:
    ax.axis("off")
fig.tight_layout()
fig.savefig(HERE / "figures" / "ocr_contact_sheet.pdf", dpi=150)
print(json.dumps({k: v for k, v in summary.items() if k != "per_image"}, indent=2))
