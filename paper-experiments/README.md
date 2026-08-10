# Experiments — GarageDB (2026-garagedb-industry5-cii)

Cinco estudios; cuatro medidos sobre artefactos reales del sistema, uno
(forecasting) sobre demanda sintética declarada. Numeración de tablas/figuras
= manuscrito v3.4.

| Estudio | Scripts | Resultados |
|---|---|---|
| Replicación (Tabla 5, Figs. 3-4) | `bench.rs` (repo, commit en `config.json`) → `run_all.py` | `bench_raw.jsonl`, `aggregated_results.json` |
| Comparación de almacenes (Tabla 4) | `bench_stores.rs`, `bench_automerge.rs` (repo) + `aggregate_stores.py` | `stores_raw.jsonl`, `automerge_raw.jsonl`, `store_comparison.json` |
| Forecasting intermitente (Tabla 6, Figs. 5-6) | `forecast_bench.py`, `forecast_extra.py` (α-sweep MASE+bias, Wilcoxon 6 pares Holm m=6, base-stock k·σ) | `forecast_results.json`, `forecast_extra.json` |
| Corpus datalogger (Tabla 7, Fig. 7) | `datalogger_analysis.py`, `datalogger_figure.py`, `verify_ts_unit.py` (unidad ts=ds verificada) | `datalogger_summary.json` |
| Piloto OCR (Sec. 6.6) | `ocr_pilot.py`, `ocr_pilot_tiled.py`, `ocr_per_label.py` (exclusión de falsos candidatos + dedupe IoU) | `ocr_pilot*.json`, `ocr_pilot_labels.csv`, `ocr_ground_truth.csv`, `ocr_per_label.json`, `figures/ocr_contact_sheet.pdf` |

## Reproducción

```bash
python -m venv .venv
.venv/Scripts/pip install -r requirements.txt   # + scipy, rapidocr-onnxruntime
set GARAGEDB_DIR=<repo garagedb>
set DATALOGGER_DIR=<corpus Zenodo 10.5281/zenodo.21871901>
.venv/Scripts/python run_all.py                 # todo end-to-end
```

Hardware de referencia: AMD Ryzen 9 5900X, 32 GB RAM, NVMe, Windows 11;
GPU R9700 no usada en los benches (CPU-only). Todas las series de tiempo de
los benches Rust se midieron back-to-back en una sesión (2026-08-10).

## Notas de integridad (post-review ronda 2)

- `ts` del datalogger está en **decisegundos** — verificado contra el canal
  `adu_track_lap_time` (pendiente 0.1000, 30k intervalos, 5 ficheros).
- El piloto OCR reporta precisión **por etiqueta física** tras excluir 5
  falsos candidatos del filtro (tornillería `H 4x16`) y fusionar 4 pares por
  IoU. Ground truth por fila en `ocr_ground_truth.csv`, re-verificable contra
  `figures/ocr_contact_sheet.pdf`.
- Automerge se reporta **por dirección de merge** (el harness mide ambas) y
  con sus bytes/evento (5.3 vs 433 de GarageDB).
