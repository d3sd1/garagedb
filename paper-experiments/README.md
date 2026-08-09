# Experiments — GarageDB (2026-garagedb-industry5-cii)

Tres estudios medidos; ningún dato simulado en el manuscrito.

## 1. Microbench de replicación (Tabla 4, Figs. 2-3)
- Fuente: `crates/core/examples/bench.rs` del repo garagedb (commit pinneado en `config.json`).
- `results/bench_raw.jsonl` (25 muestras) → `results/aggregated_results.json`.

## 2. Benchmark de forecasting intermitente (Tabla 6, Fig. 4)
- `forecast_bench.py`: 900 series/seed × 5 seeds, rolling-origin one-step,
  Croston/SBA/TSB/mean/MA(8)/naive, MASE + bias → `results/forecast_results.json`.

## 3. Corpus del datalogger (Tabla 5, Fig. 5)
- `datalogger_analysis.py` + `datalogger_figure.py` sobre `DATALOGGER_DIR`
  (corpus privado del autor; ver semántica de `ts`/`fuel` en el docstring).
- La figura selecciona la ventana EN MOVIMIENTO buscando el pico de velocidad
  en toda la sesión; falla explícitamente si no hay movimiento.

## Reproducción

```bash
python -m venv .venv
.venv/Scripts/pip install -r requirements.txt
set GARAGEDB_DIR=<ruta al repo garagedb>
set DATALOGGER_DIR=<ruta al corpus>   # opcional; sin él se omite el estudio 3
.venv/Scripts/python run_all.py          # los tres estudios end-to-end
.venv/Scripts/python run_all.py --skip-bench
```

Hardware de referencia: workstation 32 GB RAM, AMD Radeon AI PRO R9700 32 GB
(no usada en estos benches), Windows 11, Rust 1.95 release, Python 3.13.
