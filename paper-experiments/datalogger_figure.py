"""Figura real de telemetría: velocidad + ángulo de inclinación, ventana activa
del fichero más grande de la tanda 3 (delta-decode con carry-forward)."""
import json
import os
from pathlib import Path

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt

SRC = Path(os.environ.get("DATALOGGER_DIR", r"C:\Users\andre\Downloads\Archivo"))
HERE = Path(__file__).parent
LOG = SRC / "090826-00_26_TANDA3" / "LOG00057.TXT"

plt.rcParams.update({"font.family": "serif", "font.size": 10,
                     "axes.grid": True, "grid.alpha": 0.3,
                     "figure.figsize": (4.8, 3.0)})

ts, speed, lean = [], [], []
cur_speed, cur_lean = 0.0, 0.0
for line in open(LOG, encoding="utf-8", errors="ignore"):
    line = line.strip()
    if not line.startswith("{"):
        continue
    try:
        rec = json.loads(line)
    except json.JSONDecodeError:
        continue
    if "fw" in rec or "ts" not in rec:
        continue
    cur_speed = rec.get("speed_vehicle", cur_speed)
    cur_lean = rec.get("bike_lean_angle", cur_lean)
    ts.append(rec["ts"])
    speed.append(cur_speed)
    lean.append(cur_lean)

# ventana activa: 120 s alrededor del primer tramo con velocidad sostenida
start = next((i for i, v in enumerate(speed) if v > 40), 0)
t0 = ts[start]
sel = [i for i, t in enumerate(ts) if t0 <= t <= t0 + 120]
tw = [ts[i] - t0 for i in sel]

fig, ax = plt.subplots()
ax.plot(tw, [speed[i] for i in sel], color="tab:blue", label="vehicle speed (km/h)")
ax.set_xlabel("Time (s)")
ax.set_ylabel("Speed (km/h)", color="tab:blue")
ax2 = ax.twinx()
ax2.plot(tw, [lean[i] for i in sel], color="tab:red", alpha=0.7, label="lean angle (deg)")
ax2.set_ylabel("Lean angle (deg)", color="tab:red")
fig.tight_layout()
fig.savefig(HERE / "figures" / "datalogger_window.pdf")
print("muestras:", len(sel), "-> figures/datalogger_window.pdf")
