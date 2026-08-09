# Experiments — GarageDB (2026-garagedb-industry5-cii)

## Qué hay aquí

- **REAL (medido)**: microbench de replicación. Fuente: `D:/garagedb/crates/core/examples/bench.rs`.
  - `results/bench_raw.jsonl` — 25 muestras (5 tamaños × 5 repeticiones).
  - `results/aggregated_results.json` — media ± std; alimenta la Tabla 1 del paper.
  - `figures/convergence_time.pdf`, `figures/store_growth.pdf` — Figs. 1-2 del paper.
- **NO está aquí (SIMULADO en el paper)**: percepción, OCR, cold-start, estudio de campo, energía, transportes git/p2p. Todo número simulado está envuelto en `\SIM{...}` en `paper.tex`:
  ```
  grep -n "\\\\SIM{" ../paper.tex
  ```
  El `paper-corrector` trata cada `\SIM` como **bloqueante** para promoción a `to_send/`. Se sustituyen por datos reales conforme el despliegue avance.

## Reproducción

```bash
python -m venv .venv
.venv/Scripts/pip install -r requirements.txt
set GARAGEDB_DIR=D:/garagedb
.venv/Scripts/python run_all.py          # re-ejecuta bench (cargo) + agrega + figuras
.venv/Scripts/python run_all.py --skip-bench   # solo agrega + figuras
```

Hardware de referencia: workstation 32 GB RAM, AMD Radeon AI PRO R9700 32 GB (no usada en el bench de replicación), Windows 11, Rust 1.95 release.
