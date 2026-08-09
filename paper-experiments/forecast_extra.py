"""Extensiones del benchmark de forecasting (bloque de días del meta-review):
  A) α-sweep: estabilidad del orden Croston/SBA/TSB para α ∈ {0.05,0.1,0.15,0.2}
  B) Test pareado entre series: Wilcoxon signed-rank con corrección de Holm
     sobre los MASE por serie (4.500 series), Croston vs SBA vs TSB vs Mean
  C) Simulación de política de inventario: base-stock periódica (R=1, L=2),
     fill rate vs inventario medio en mano barriendo el multiplicador de
     seguridad — la métrica de servicio que la evaluación de accuracy no da.
Salida: results/forecast_extra.json + figures/fillrate_tradeoff.pdf
"""
import json
from pathlib import Path

import numpy as np
from scipy import stats

import forecast_bench as fb

HERE = Path(__file__).parent
OUT = HERE / "results" / "forecast_extra.json"
SEEDS = fb.SEEDS
TRAIN = fb.TRAIN
LEAD = 2  # semanas de lead time del proveedor típico


def gen_all(seed):
    rng = np.random.default_rng(seed)
    return [fb.gen_sku(rng)[0] for _ in range(fb.N_SKUS)]


def per_series_mase(y, f):
    return fb.mase(y, f)


# ---------- A) alpha sweep ----------
def alpha_sweep():
    out = {}
    for alpha in [0.05, 0.1, 0.15, 0.2]:
        rows = {m: [] for m in ["croston", "sba", "tsb"]}
        for seed in SEEDS:
            series = gen_all(seed)
            for y in series:
                for m in rows:
                    if m == "tsb":
                        f = fb.tsb(y, alpha=alpha, beta=alpha)
                    else:
                        f = fb.croston(y, alpha=alpha, variant=m)
                    v = per_series_mase(y, f)
                    if np.isfinite(v):
                        rows[m].append(v)
        out[str(alpha)] = {
            m: {"mase_mean": round(float(np.mean(v)), 3)} for m, v in rows.items()
        }
        order = sorted(rows, key=lambda m: np.mean(rows[m]))
        out[str(alpha)]["order"] = order
    return out


# ---------- B) paired tests over series ----------
def paired_tests():
    methods = ["croston", "sba", "tsb", "mean"]
    per_method = {m: [] for m in methods}
    for seed in SEEDS:
        series = gen_all(seed)
        for y in series:
            vals = {}
            for m in methods:
                f = fb.METHODS[m](y)
                vals[m] = per_series_mase(y, f)
            if all(np.isfinite(v) for v in vals.values()):
                for m in methods:
                    per_method[m].append(vals[m])
    n = len(per_method["croston"])
    arrs = {m: np.array(v) for m, v in per_method.items()}

    fried = stats.friedmanchisquare(*[arrs[m] for m in methods])
    pairs = [("sba", "croston"), ("sba", "tsb"), ("sba", "mean"), ("croston", "tsb")]
    raw = []
    for a, b in pairs:
        w = stats.wilcoxon(arrs[a], arrs[b])
        raw.append((f"{a}_vs_{b}", float(w.pvalue),
                    round(float(np.median(arrs[a] - arrs[b])), 4)))
    # Holm
    order = np.argsort([p for _, p, _ in raw])
    m_tests = len(raw)
    holm = {}
    for rank, idx in enumerate(order):
        name, p, med = raw[idx]
        holm[name] = {"p_holm": min(1.0, p * (m_tests - rank)),
                      "median_diff_mase": med}
    return {
        "n_series": n,
        "friedman_chi2": round(float(fried.statistic), 1),
        "friedman_p": float(fried.pvalue),
        "wilcoxon_holm": holm,
    }


# ---------- C) inventory policy simulation ----------
def simulate_policy(y, forecast, mult):
    """Base-stock periódica R=1, lead L=2, sobre la ventana de test.
    S_t = forecast_t * (L+1) * mult. Devuelve (fill_rate, on_hand_medio)."""
    on_hand = float(y[:TRAIN].mean()) * (LEAD + 1) * mult  # arranque razonable
    pipeline = [0.0] * LEAD
    served = 0.0
    demanded = 0.0
    hold = []
    for t in range(TRAIN, len(y)):
        on_hand += pipeline.pop(0)  # llega el pedido de hace L semanas
        d = y[t]
        demanded += d
        s = min(d, on_hand)
        served += s
        on_hand -= s
        target = forecast[t] * (LEAD + 1) * mult
        order = max(0.0, target - on_hand - sum(pipeline))
        pipeline.append(order)
        hold.append(on_hand)
    fr = served / demanded if demanded > 0 else np.nan
    return fr, float(np.mean(hold))


def policy_curves():
    methods = ["croston", "sba", "tsb", "mean"]
    mults = [0.6, 0.8, 1.0, 1.25, 1.5, 2.0, 2.5]
    curves = {m: [] for m in methods}
    for mult in mults:
        acc = {m: ([], []) for m in methods}
        for seed in SEEDS:
            series = gen_all(seed)
            for y in series:
                if y[TRAIN:].sum() == 0:
                    continue
                for m in methods:
                    f = fb.METHODS[m](y)
                    fr, oh = simulate_policy(y, f, mult)
                    if np.isfinite(fr):
                        acc[m][0].append(fr)
                        acc[m][1].append(oh)
        for m in methods:
            curves[m].append({
                "mult": mult,
                "fill_rate": round(float(np.mean(acc[m][0])), 4),
                "avg_on_hand": round(float(np.mean(acc[m][1])), 2),
            })
    return curves


def plot_curves(curves):
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    plt.rcParams.update({"font.family": "serif", "font.size": 10,
                         "axes.grid": True, "grid.alpha": 0.3,
                         "figure.figsize": (4.8, 3.2)})
    fig, ax = plt.subplots()
    styles = {"croston": "o-", "sba": "s-", "tsb": "^-", "mean": "d--"}
    for m, pts in curves.items():
        ax.plot([p["avg_on_hand"] for p in pts], [p["fill_rate"] for p in pts],
                styles[m], label=m.upper() if m != "mean" else "Mean", markersize=4)
    ax.set_xlabel("Average on-hand inventory (units)")
    ax.set_ylabel("Fill rate")
    ax.legend(fontsize=8)
    fig.tight_layout()
    fig.savefig(HERE / "figures" / "fillrate_tradeoff.pdf")


def main():
    result = {
        "alpha_sweep": alpha_sweep(),
        "paired_tests": paired_tests(),
        "policy": {"lead_time_weeks": LEAD, "review_weeks": 1},
    }
    curves = policy_curves()
    result["policy"]["curves"] = curves
    OUT.write_text(json.dumps(result, indent=2), encoding="utf-8")
    plot_curves(curves)
    print(json.dumps({k: v for k, v in result.items() if k != "policy"}, indent=2))
    print("-> results/forecast_extra.json, figures/fillrate_tradeoff.pdf")


if __name__ == "__main__":
    main()
