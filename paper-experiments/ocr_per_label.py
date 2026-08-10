"""Recomputación per-label del piloto OCR (review R2: N7/N8 junior, N9-N11 métodos):
1. Excluye falsos candidatos del filtro (verdad NO matricial: tornillería H 4x16...).
2. Dedupe por IoU de bounding boxes (>0.3) quedándose con la lectura de máxima
   confianza por etiqueta física.
3. Precisión exact-match POR ETIQUETA, global y por umbral de confianza.
Entradas: results/ocr_pilot_labels.csv (detecciones) + results/ocr_ground_truth.csv
(verdad por (text, conf), anotación del operador re-verificable en el contact sheet).
"""
import csv
import json
from pathlib import Path

HERE = Path(__file__).parent
R = HERE / "results"

# cargar detecciones con bbox
dets = []
with open(R / "ocr_pilot_labels.csv", encoding="utf-8") as f:
    for row in csv.DictReader(f):
        bbox = json.loads(row["bbox"]) if row["bbox"].startswith("[") else eval(row["bbox"])
        dets.append({"image": row["image"], "text": row["text"],
                     "conf": float(row["conf"]), "bbox": bbox})

# cargar verdad, emparejar por (text, conf redondeada a 2)
truth = {}
with open(R / "ocr_ground_truth.csv", encoding="utf-8") as f:
    for row in csv.DictReader(f):
        truth[(row["text"], round(float(row["conf"]), 2))] = (
            row["truth"], row["truth_is_matrix"] == "1")

matched = 0
for d in dets:
    key = (d["text"], round(d["conf"], 2))
    if key in truth:
        d["truth"], d["is_matrix"] = truth[key]
        matched += 1
    else:
        d["truth"], d["is_matrix"] = None, None
assert matched == len(dets), f"verdad sin emparejar: {len(dets) - matched}"

# 1) excluir candidatos cuya verdad no es matricial (falso positivo del filtro)
filter_fp = [d for d in dets if not d["is_matrix"]]
matrix_dets = [d for d in dets if d["is_matrix"]]

# 2) dedupe IoU dentro de cada imagen
def iou(a, b):
    ax0, ay0, ax1, ay1 = a
    bx0, by0, bx1, by1 = b
    ix = max(0, min(ax1, bx1) - max(ax0, bx0))
    iy = max(0, min(ay1, by1) - max(ay0, by0))
    inter = ix * iy
    if inter == 0:
        return 0.0
    area = (ax1 - ax0) * (ay1 - ay0) + (bx1 - bx0) * (by1 - by0) - inter
    return inter / area

labels = []  # grupos por etiqueta física
for d in sorted(matrix_dets, key=lambda x: -x["conf"]):
    placed = False
    for g in labels:
        if g[0]["image"] == d["image"] and iou(g[0]["bbox"], d["bbox"]) > 0.3:
            g.append(d)
            placed = True
            break
    if not placed:
        labels.append([d])

per_label = []
for g in labels:
    best = g[0]  # máxima confianza (orden de inserción)
    per_label.append({
        "image": best["image"], "read": best["text"], "truth": best["truth"],
        "conf": best["conf"], "n_readings": len(g),
        "correct": best["text"] == best["truth"].replace(" ", "").upper()
        if best["truth"] else False,
    })

n = len(per_label)
correct = sum(1 for l in per_label if l["correct"])
hi = [l for l in per_label if l["conf"] >= 0.75]
lo = [l for l in per_label if l["conf"] < 0.75]
merged_pairs = sum(1 for g in labels if len(g) > 1)

summary = {
    "detections_total": len(dets),
    "filter_false_positives_non_matrix": len(filter_fp),
    "filter_fp_texts": sorted({d["text"] for d in filter_fp}),
    "matrix_detections": len(matrix_dets),
    "physical_labels_after_iou_dedupe": n,
    "iou_merged_groups": merged_pairs,
    "per_label_exact_correct": correct,
    "per_label_precision": round(correct / n, 3),
    "tau_0.75": {"n": len(hi), "correct": sum(1 for l in hi if l["correct"]),
                 "precision": round(sum(1 for l in hi if l["correct"]) / len(hi), 3) if hi else None},
    "below_0.75": {"n": len(lo), "correct": sum(1 for l in lo if l["correct"]),
                   "precision": round(sum(1 for l in lo if l["correct"]) / len(lo), 3) if lo else None},
    "per_image": {},
}
for l in per_label:
    summary["per_image"].setdefault(l["image"], 0)
    summary["per_image"][l["image"]] += 1

(R / "ocr_per_label.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
print(json.dumps(summary, indent=2))
