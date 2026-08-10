"""Verificación empírica de la unidad de `ts` (objeción N8/R3), método 2:
ancla física con el motor. `engine_revolutions` es un contador acumulado de
vueltas; `rpm` son revoluciones por minuto. Con el motor a régimen sostenido:

    d(engine_revolutions)/d(ts)  ≈  rpm/60      si ts está en segundos
                                  ≈  rpm/600    si ts está en décimas

ratio = pendiente_medida / (rpm_medio/60):  ≈1.0 → segundos; ≈0.1 → décimas.
"""
import json
import os
from pathlib import Path

import numpy as np

SRC = Path(os.environ.get("DATALOGGER_DIR", r"C:\Users\andre\Downloads\Archivo"))

results = []
for session in ["stint 1", "stint 3"]:
    sdir = SRC / session
    if not sdir.exists():
        continue
    files = sorted(sdir.glob("LOG*.TXT"), key=lambda p: -p.stat().st_size)[:6]
    for f in files:
        rows = []  # (ts, revs, rpm)
        cur_rpm = 0
        for line in open(f, encoding="utf-8", errors="ignore"):
            line = line.strip()
            if not line.startswith("{"):
                continue
            try:
                rec = json.loads(line)
            except json.JSONDecodeError:
                continue
            if "fw" in rec or "ts" not in rec:
                continue
            cur_rpm = rec.get("rpm", cur_rpm)
            if "engine_revolutions" in rec and rec["engine_revolutions"] > 0:
                rows.append((rec["ts"], rec["engine_revolutions"], cur_rpm))
        if len(rows) < 50:
            continue
        # tramos con motor girando estable (rpm > 1500) y muestras consecutivas
        ratios = []
        for i in range(1, len(rows)):
            t0, r0, _ = rows[i - 1]
            t1, r1, rpm = rows[i]
            dt, dr = t1 - t0, r1 - r0
            if dt <= 0 or dr <= 0 or rpm < 1500:
                continue
            slope = dr / dt                 # vueltas por unidad de ts
            expected = rpm / 60.0           # vueltas por segundo real
            ratios.append(slope / expected)
        if len(ratios) < 10:
            continue
        med = float(np.median(ratios))
        results.append({"session": session, "file": f.name,
                        "n_intervals": len(ratios), "ratio_median": round(med, 3)})

print(json.dumps(results, indent=2))
meds = [r["ratio_median"] for r in results]
if meds:
    overall = float(np.median(meds))
    print(f"\nMEDIANA global del ratio: {overall:.3f}")
    if 0.8 < overall < 1.25:
        print("=> ts esta en SEGUNDOS (la hipotesis del factor 10 queda refutada)")
    elif 0.07 < overall < 0.14:
        print("=> ts esta en DECIMAS DE SEGUNDO (hipotesis N8 confirmada: corregir x10)")
    else:
        print("=> NO concluyente")
