"""Orquestador: ejecuta el microbench de replicación de GarageDB y regenera
resultados agregados y figuras. Requiere el repo del software en GARAGEDB_DIR."""
import json
import os
import statistics
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).parent
GARAGEDB_DIR = Path(os.environ.get("GARAGEDB_DIR", "D:/garagedb"))
RAW = HERE / "results" / "bench_raw.jsonl"
AGG = HERE / "results" / "aggregated_results.json"


def run_bench() -> None:
    out = subprocess.run(
        ["cargo", "run", "--release", "--example", "bench"],
        cwd=GARAGEDB_DIR, capture_output=True, text=True, check=True,
    )
    lines = [l for l in out.stdout.splitlines() if l.startswith("{")]
    RAW.parent.mkdir(parents=True, exist_ok=True)
    RAW.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"bench: {len(lines)} muestras -> {RAW}")


def aggregate() -> dict:
    rows = [json.loads(l) for l in RAW.read_text(encoding="utf-8").splitlines() if l.strip()]
    by_n: dict[int, list[dict]] = {}
    for r in rows:
        by_n.setdefault(r["n"], []).append(r)
    agg = {"replication": []}
    for n in sorted(by_n):
        g = by_n[n]
        tv = [r["t_fold_verify_ms"] for r in g]
        tp = [r["t_parse_only_ms"] for r in g]
        sb = [r["store_bytes"] for r in g]
        agg["replication"].append({
            "n_events": n,
            "t_fold_verify_ms_mean": round(statistics.mean(tv), 2),
            "t_fold_verify_ms_std": round(statistics.stdev(tv), 2),
            "t_parse_only_ms_mean": round(statistics.mean(tp), 2),
            "t_parse_only_ms_std": round(statistics.stdev(tp), 2),
            "store_bytes_mean": int(statistics.mean(sb)),
            "bytes_per_event": round(statistics.mean(sb) / n, 1),
            "signature_overhead_factor": round(statistics.mean(tv) / statistics.mean(tp), 1),
        })
    AGG.write_text(json.dumps(agg, indent=2), encoding="utf-8")
    print(f"agregado -> {AGG}")
    return agg


def main() -> None:
    if "--skip-bench" not in sys.argv:
        run_bench()
    aggregate()
    subprocess.run([sys.executable, str(HERE / "generate_figures.py")], check=True)
    subprocess.run([sys.executable, str(HERE / "forecast_bench.py")], check=True)
    if os.environ.get("DATALOGGER_DIR"):
        subprocess.run([sys.executable, str(HERE / "datalogger_analysis.py")], check=True)
        subprocess.run([sys.executable, str(HERE / "datalogger_figure.py")], check=True)


if __name__ == "__main__":
    main()
