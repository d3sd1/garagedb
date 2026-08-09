"""Figura real de telemetría: velocidad + ángulo de inclinación, ventana EN
MOVIMIENTO buscada sobre TODOS los ficheros de la sesión (delta-decode con
carry-forward). Sin fallback silencioso: si ningún fichero supera el umbral,
el script FALLA con mensaje explícito."""
import json
import os
import sys
from pathlib import Path

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt

SRC = Path(os.environ.get("DATALOGGER_DIR", "./datalogger-raw"))
SESSION = SRC / "090826-00_26_TANDA3"
HERE = Path(__file__).parent
SPEED_THRESHOLD = 60.0  # km/h: claramente en pista, no maniobra de paddock
WINDOW_S = 120.0

plt.rcParams.update({"font.family": "serif", "font.size": 10,
                     "axes.grid": True, "grid.alpha": 0.3,
                     "figure.figsize": (4.8, 3.0)})


def decode(path: Path):
    """Delta-decode: (ts, speed, lean) con carry-forward."""
    ts, speed, lean = [], [], []
    cur_s, cur_l = 0.0, 0.0
    for line in open(path, encoding="utf-8", errors="ignore"):
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            rec = json.loads(line)
        except json.JSONDecodeError:
            continue
        if "fw" in rec or "ts" not in rec:
            continue
        cur_s = rec.get("speed_vehicle", cur_s)
        cur_l = rec.get("bike_lean_angle", cur_l)
        ts.append(rec["ts"])
        speed.append(cur_s)
        lean.append(cur_l)
    return ts, speed, lean


# buscar el fichero con la ventana de mayor actividad de toda la sesión
best = None  # (max_speed, path, ts, speed, lean, idx_of_max)
for f in sorted(SESSION.glob("LOG*.TXT")):
    ts, speed, lean = decode(f)
    if not speed:
        continue
    m = max(speed)
    if m >= SPEED_THRESHOLD and (best is None or m > best[0]):
        best = (m, f, ts, speed, lean, speed.index(m))

if best is None:
    sys.exit(f"ERROR: ningún fichero de {SESSION} supera {SPEED_THRESHOLD} km/h; "
             "no se genera figura con el vehículo parado.")

m, path, ts, speed, lean, imax = best
# ventana centrada en el pico de velocidad
t_peak = ts[imax]
t0 = max(ts[0], t_peak - WINDOW_S / 2)
sel = [i for i, t in enumerate(ts) if t0 <= t <= t0 + WINDOW_S]
tw = [ts[i] - t0 for i in sel]
sp = [speed[i] for i in sel]
ln = [lean[i] for i in sel]

assert max(sp) >= SPEED_THRESHOLD, "la ventana seleccionada no está en movimiento"

fig, ax = plt.subplots()
ax.plot(tw, sp, color="tab:blue", label="vehicle speed (km/h)")
ax.set_xlabel("Time (s)")
ax.set_ylabel("Speed (km/h)", color="tab:blue")
ax2 = ax.twinx()
ax2.plot(tw, ln, color="tab:red", alpha=0.7, label="lean angle (deg)")
ax2.set_ylabel("Lean angle (deg)", color="tab:red")
fig.tight_layout()
fig.savefig(HERE / "figures" / "datalogger_window.pdf")
print(f"fichero {path.name}, pico {m:.1f} km/h, {len(sel)} muestras "
      f"-> figures/datalogger_window.pdf")
