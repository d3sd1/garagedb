"""Figuras del paper (PDF vectorial, serif, 10pt) desde aggregated_results.json."""
import json
from pathlib import Path

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt

HERE = Path(__file__).parent
AGG = json.loads((HERE / "results" / "aggregated_results.json").read_text(encoding="utf-8"))
FIG = HERE / "figures"
FIG.mkdir(exist_ok=True)

plt.rcParams.update({
    "font.family": "serif",
    "font.size": 10,
    "axes.grid": True,
    "grid.alpha": 0.3,
    "figure.figsize": (4.6, 3.0),
})

rep = AGG["replication"]
n = [r["n_events"] for r in rep]

# Figura 1: tiempo de convergencia vs eventos divergentes
tv = [r["t_fold_verify_ms_mean"] for r in rep]
tv_s = [r["t_fold_verify_ms_std"] for r in rep]
tp = [r["t_parse_only_ms_mean"] for r in rep]
tp_s = [r["t_parse_only_ms_std"] for r in rep]

fig, ax = plt.subplots()
ax.errorbar(n, tv, yerr=tv_s, marker="o", label="fold + ed25519 verify")
ax.errorbar(n, tp, yerr=tp_s, marker="s", label="fold, parse-only")
ax.set_xscale("log")
ax.set_yscale("log")
ax.set_xlabel("Divergent events (two replicas, folder transport)")
ax.set_ylabel("Convergence time (ms)")
ax.legend()
fig.tight_layout()
fig.savefig(FIG / "convergence_time.pdf")
plt.close(fig)

# Figura 2: tamaño del almacén
bpe = [r["bytes_per_event"] for r in rep]
sb = [r["store_bytes_mean"] / 1024 for r in rep]
fig, ax = plt.subplots()
ax.plot(n, sb, marker="o", color="tab:green", label="store size (KiB)")
ax.set_xlabel("Events in store")
ax.set_ylabel("Store size (KiB)")
ax2 = ax.twinx()
ax2.plot(n, bpe, marker="s", color="tab:gray", linestyle="--", label="bytes/event")
ax2.set_ylabel("Bytes per event")
ax2.set_ylim(0, max(bpe) * 1.3)
lines = ax.get_lines() + ax2.get_lines()
ax.legend(lines, [l.get_label() for l in lines], loc="upper left")
fig.tight_layout()
fig.savefig(FIG / "store_growth.pdf")
plt.close(fig)

print("figuras ->", FIG)
