"""Extensiones del benchmark de forecasting (v2, tras review ronda 2):
  A) α-sweep con MASE **y BIAS** por método (la magnitud del claim; R2-N3/R3)
  B) Tests pareados: familia COMPLETA de 6 pares sobre MASE y sobre BIAS,
     Wilcoxon signed-rank con Holm m=6 y running-max (R3-N12)
  C) Política base-stock k·σ: S = μ_LT + k·σ_LT (sesgo puede sacar de la
     frontera, no solo moverte por ella; R3-N14), barrido de k hasta la
     región de servicio alto (fill ~0.99), on-hand + pipeline reportados.
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
LEAD = 2  # semanas de lead time
REVIEW = 1


def gen_all(seed):
    rng = np.random.default_rng(seed)
    return [fb.gen_sku(rng)[0] for _ in range(fb.N_SKUS)]


# ---------- A) alpha sweep: MASE y BIAS ----------
def alpha_sweep():
    out = {}
    for alpha in [0.05, 0.1, 0.15, 0.2]:
        rows_mase = {m: [] for m in ["croston", "sba", "tsb"]}
        rows_bias = {m: [] for m in ["croston", "sba", "tsb"]}
        for seed in SEEDS:
            for y in gen_all(seed):
                for m in rows_mase:
                    if m == "tsb":
                        f = fb.tsb(y, alpha=alpha, beta=alpha)  # beta=alpha, declarado
                    else:
                        f = fb.croston(y, alpha=alpha, variant=m)
                    v = fb.mase(y, f)
                    if np.isfinite(v):
                        rows_mase[m].append(v)
                    rows_bias[m].append(fb.bias(y, f))
        entry = {}
        for m in rows_mase:
            entry[m] = {
                "mase_mean": round(float(np.mean(rows_mase[m])), 3),
                "bias_mean": round(float(np.mean(rows_bias[m])), 3),
            }
        entry["order_mase"] = sorted(rows_mase, key=lambda m: np.mean(rows_mase[m]))
        entry["order_abs_bias"] = sorted(rows_bias, key=lambda m: abs(np.mean(rows_bias[m])))
        out[str(alpha)] = entry
    return out


# ---------- B) paired tests: 6 pares, MASE y BIAS ----------
def holm(pvals: dict) -> dict:
    items = sorted(pvals.items(), key=lambda kv: kv[1])
    m = len(items)
    out = {}
    running = 0.0
    for rank, (name, p) in enumerate(items):
        adj = min(1.0, p * (m - rank))
        running = max(running, adj)  # running-max (monotonia de Holm)
        out[name] = running
    return out


def paired_tests():
    methods = ["croston", "sba", "tsb", "mean"]
    per_mase = {m: [] for m in methods}
    per_bias = {m: [] for m in methods}
    for seed in SEEDS:
        for y in gen_all(seed):
            vals_mase, vals_bias = {}, {}
            for m in methods:
                f = fb.METHODS[m](y)
                vals_mase[m] = fb.mase(y, f)
                vals_bias[m] = fb.bias(y, f)
            if all(np.isfinite(v) for v in vals_mase.values()):
                for m in methods:
                    per_mase[m].append(vals_mase[m])
                    per_bias[m].append(vals_bias[m])
    n = len(per_mase["croston"])
    arr_mase = {m: np.array(v) for m, v in per_mase.items()}
    arr_bias = {m: np.array(v) for m, v in per_bias.items()}

    pairs = [(a, b) for i, a in enumerate(methods) for b in methods[i + 1:]]
    result = {"n_series": n, "n_pairs": len(pairs)}
    for tag, arrs, transform in [
        ("mase", arr_mase, lambda x: x),
        ("abs_bias", arr_bias, np.abs),  # magnitud del sesgo por serie
    ]:
        fried = stats.friedmanchisquare(*[transform(arrs[m]) for m in methods])
        raw_p, med = {}, {}
        for a, b in pairs:
            da, db = transform(arrs[a]), transform(arrs[b])
            w = stats.wilcoxon(da, db)
            key = f"{a}_vs_{b}"
            raw_p[key] = float(w.pvalue)
            med[key] = round(float(np.median(da - db)), 4)
        adj = holm(raw_p)
        result[tag] = {
            "friedman_chi2": round(float(fried.statistic), 1),
            "friedman_p": float(fried.pvalue),
            "pairs": {k: {"p_holm": (adj[k] if adj[k] > 1e-300 else "<1e-300"),
                          "median_diff": med[k]} for k in raw_p},
        }
    return result


# ---------- C) base-stock k-sigma ----------
def simulate_policy_ksigma(y, forecast, k, sigma_lt):
    """Base-stock periodica R=1, L=2: S_t = forecast_t*(L+R) + k*sigma_LT.
    sigma_LT estimado de los residuos in-sample. Devuelve (fill, on_hand+pipeline)."""
    horizon = LEAD + REVIEW
    on_hand = float(y[:TRAIN].mean()) * horizon + k * sigma_lt
    pipeline = [0.0] * LEAD
    served = demanded = 0.0
    inv = []
    for t in range(TRAIN, len(y)):
        on_hand += pipeline.pop(0)
        d = y[t]
        demanded += d
        s = min(d, on_hand)
        served += s
        on_hand -= s
        target = forecast[t] * horizon + k * sigma_lt
        order = max(0.0, target - on_hand - sum(pipeline))
        pipeline.append(order)
        inv.append(on_hand + sum(pipeline))  # inversion total, no solo on-hand
    fr = served / demanded if demanded > 0 else np.nan
    return fr, float(np.mean(inv))


def policy_curves():
    methods = ["croston", "sba", "tsb", "mean"]
    ks = [0.0, 0.5, 1.0, 1.5, 2.0, 3.0, 4.0, 6.0]
    curves = {m: [] for m in methods}
    skipped = 0
    for k in ks:
        acc = {m: ([], []) for m in methods}
        for seed in SEEDS:
            for y in gen_all(seed):
                if y[TRAIN:].sum() == 0:
                    if k == ks[0]:
                        skipped += 1
                    continue
                # sigma de residuos one-step in-sample del propio metodo,
                # escalado a lead time por sqrt(horizonte)
                for m in methods:
                    f = fb.METHODS[m](y)
                    resid = y[1:TRAIN] - f[1:TRAIN]
                    sigma_lt = float(np.std(resid)) * np.sqrt(LEAD + REVIEW)
                    fr, inv = simulate_policy_ksigma(y, f, k, sigma_lt)
                    if np.isfinite(fr):
                        acc[m][0].append(fr)
                        acc[m][1].append(inv)
        for m in methods:
            curves[m].append({
                "k": k,
                "fill_rate": round(float(np.mean(acc[m][0])), 4),
                "fill_rate_std": round(float(np.std(acc[m][0])), 4),
                "avg_inventory": round(float(np.mean(acc[m][1])), 2),
            })
    return curves, skipped


def plot_curves(curves):
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    plt.rcParams.update({"font.family": "serif", "font.size": 10,
                         "axes.grid": True, "grid.alpha": 0.3,
                         "figure.figsize": (4.8, 3.2)})
    fig, ax = plt.subplots()
    styles = {"croston": "o-", "sba": "s-", "tsb": "^-", "mean": "d--"}
    names = {"croston": "Croston", "sba": "SBA", "tsb": "TSB", "mean": "Mean"}
    for m, pts in curves.items():
        ax.plot([p["avg_inventory"] for p in pts], [p["fill_rate"] for p in pts],
                styles[m], label=names[m], markersize=4)
    ax.axhline(0.95, color="gray", lw=0.8, ls=":")
    ax.set_xlabel("Average inventory position (on-hand + pipeline, units)")
    ax.set_ylabel("Fill rate")
    ax.legend(fontsize=8, loc="lower right")
    fig.tight_layout()
    fig.savefig(HERE / "figures" / "fillrate_tradeoff.pdf")


def main():
    curves, skipped = policy_curves()
    result = {
        "alpha_sweep": alpha_sweep(),
        "paired_tests": paired_tests(),
        "policy": {"design": "base-stock S = mu_LT + k*sigma_LT, R=1, L=2, k swept",
                   "lead_time_weeks": LEAD, "review_weeks": REVIEW,
                   "series_skipped_no_test_demand": skipped,
                   "curves": curves},
    }
    OUT.write_text(json.dumps(result, indent=2), encoding="utf-8")
    plot_curves(curves)
    print(json.dumps({"alpha_sweep": result["alpha_sweep"],
                      "paired": {k: v for k, v in result["paired_tests"].items() if k != "n_series"}},
                     indent=2)[:3000])
    print("-> results/forecast_extra.json, figures/fillrate_tradeoff.pdf")


if __name__ == "__main__":
    main()
