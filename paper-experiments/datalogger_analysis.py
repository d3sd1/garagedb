"""Caracterización del workload real del datalogger (día de circuito 2026-08-09).

Entrada: carpetas de tandas con LOGxxxxx.TXT en JSONL delta-encoded
(primera línea de datos = snapshot completo; siguientes = solo canales que cambian).
Salida: results/datalogger_summary.json + figura de telemetría representativa.
"""
import json
import os
from pathlib import Path

SRC = Path(os.environ.get("DATALOGGER_DIR", r"C:\Users\andre\Downloads\Archivo"))
HERE = Path(__file__).parent
OUT = HERE / "results" / "datalogger_summary.json"


def parse_file(path: Path) -> dict:
    n_lines = 0
    n_channels = set()
    t_min, t_max = None, None
    fuel_max = 0.0
    speed_max = 0.0
    fw = None
    with open(path, encoding="utf-8", errors="ignore") as f:
        for line in f:
            line = line.strip()
            if not line or not line.startswith("{"):
                continue
            try:
                rec = json.loads(line)
            except json.JSONDecodeError:
                continue
            if "fw" in rec:
                fw = rec["fw"]
                continue
            n_lines += 1
            n_channels.update(rec.keys())
            ts = rec.get("ts")
            if ts is not None:
                t_min = ts if t_min is None else min(t_min, ts)
                t_max = ts if t_max is None else max(t_max, ts)
            if "fuel_consumed_l" in rec:
                fuel_max = max(fuel_max, rec["fuel_consumed_l"])
            if "speed_vehicle" in rec:
                speed_max = max(speed_max, rec["speed_vehicle"])
    return {
        "file": path.name,
        "bytes": path.stat().st_size,
        "samples": n_lines,
        "duration_s": round((t_max - t_min), 1) if t_min is not None else 0.0,
        "channels": len(n_channels - {"ts"}),
        "fuel_l": round(fuel_max, 3),
        "speed_max": round(speed_max, 1),
        "fw": fw,
    }


def main() -> None:
    sessions = {}
    for d in sorted(SRC.iterdir()):
        if not d.is_dir() or d.name.startswith("__"):
            continue
        files = sorted(d.glob("LOG*.TXT"))
        if not files:
            continue
        parsed = [parse_file(p) for p in files]
        sessions[d.name] = {
            "n_files": len(parsed),
            "total_mb": round(sum(p["bytes"] for p in parsed) / 1e6, 1),
            "total_samples": sum(p["samples"] for p in parsed),
            "total_logged_s": round(sum(p["duration_s"] for p in parsed), 0),
            "channels": max((p["channels"] for p in parsed), default=0),
            "fuel_l": round(sum(p["fuel_l"] for p in parsed), 2),
            "speed_max_kmh": max((p["speed_max"] for p in parsed), default=0),
            "fw": next((p["fw"] for p in parsed if p["fw"]), None),
            "largest_file": max(parsed, key=lambda p: p["bytes"])["file"],
        }
    OUT.parent.mkdir(exist_ok=True)
    OUT.write_text(json.dumps(sessions, indent=2), encoding="utf-8")
    print(json.dumps(sessions, indent=2))


if __name__ == "__main__":
    main()
