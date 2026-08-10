"""Caracterización del corpus del datalogger (recuperado el día de circuito
2026-08-09).

Entrada: DATALOGGER_DIR con carpetas de tandas conteniendo LOGxxxxx.TXT en
JSONL delta-encoded (línea de boot + snapshot completo; después solo canales
que cambian).

Notas de semántica (ver review R3):
- `ts` es un contador POR ARRANQUE en DECISEGUNDOS (verificado empiricamente:
  d(adu_track_lap_time)/d(ts) = 0.1000 exacto sobre 30k intervalos, ver
  verify_ts_unit.py). Los spans se convierten a segundos dividiendo por 10. `logged_s_sum` es la SUMA de
  los spans por fichero: tiempo total de logging acumulado a través de todos
  los encendidos del equipo, NO la duración de pared de una sesión. Los
  ficheros se acumulan en la SD a través de múltiples encendidos (garaje,
  calentamiento, tandas).
- `fuel_consumed_l` es un contador acumulativo POR ARRANQUE (arranca ~0 en
  cada boot). El máximo por fichero = combustible de ese arranque; la suma
  por sesión ASUME un arranque por fichero y se reporta como estimación.
- "records" son líneas delta, no muestras completas de 97 canales;
  `mean_channels_per_record` da la densidad real.
"""
import json
import os
from pathlib import Path

SRC = Path(os.environ.get("DATALOGGER_DIR", "./datalogger-raw"))
HERE = Path(__file__).parent
OUT = HERE / "results" / "datalogger_summary.json"


def parse_file(path: Path) -> dict:
    n_lines = 0
    n_keys_total = 0
    channels = set()
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
            keys = rec.keys() - {"ts"}
            n_keys_total += len(keys)
            channels.update(keys)
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
        "records": n_lines,
        "span_s": round((t_max - t_min) / 10.0, 1) if t_min is not None else 0.0,  # ts en decisegundos (verificado en verify_ts_unit.py)
        "keys_total": n_keys_total,
        "channels": len(channels),
        "fuel_l_boot_max": round(fuel_max, 3),
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
        total_records = sum(p["records"] for p in parsed)
        logged = sum(p["span_s"] for p in parsed)
        sessions[d.name] = {
            "n_files": len(parsed),
            "total_mb": round(sum(p["bytes"] for p in parsed) / 1e6, 1),
            "total_records": total_records,
            "logged_s_sum": round(logged, 0),
            "mean_record_rate_hz": round(total_records / logged, 1) if logged else 0,
            "channels_union": max((p["channels"] for p in parsed), default=0),
            "mean_channels_per_record": round(
                sum(p["keys_total"] for p in parsed) / total_records, 1
            ) if total_records else 0,
            "fuel_l_sum_of_boot_max_ESTIMATE": round(
                sum(p["fuel_l_boot_max"] for p in parsed), 2
            ),
            "speed_max_kmh": max((p["speed_max"] for p in parsed), default=0),
            "fw": next((p["fw"] for p in parsed if p["fw"]), None),
            "largest_file": max(parsed, key=lambda p: p["bytes"])["file"],
        }
    OUT.parent.mkdir(exist_ok=True)
    OUT.write_text(json.dumps(sessions, indent=2), encoding="utf-8")
    print(json.dumps(sessions, indent=2))


if __name__ == "__main__":
    main()
