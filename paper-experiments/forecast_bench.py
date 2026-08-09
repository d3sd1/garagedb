"""Benchmark REAL de previsión de demanda intermitente (datos sintéticos declarados).

Genera 900 series de demanda semanal (104 semanas) cubriendo los cuatro
cuadrantes de Syntetos-Boylan (smooth / intermittent / erratic / lumpy),
evalúa Croston, SBA, TSB, naive y media móvil con MASE y sesgo (ME),
sobre 5 seeds fijos. Salida: results/forecast_results.json + figura.
"""
import json
from pathlib import Path

import numpy as np

HERE = Path(__file__).parent
SEEDS = [42, 123, 456, 789, 1024]
N_SKUS = 900
WEEKS = 104
TRAIN = 78
ALPHA = 0.1
BETA = 0.1


def gen_sku(rng: np.random.Generator) -> tuple[np.ndarray, str]:
    """Serie de demanda de un SKU. Cuadrante por (ADI, CV2) objetivo."""
    quadrant = rng.choice(["smooth", "intermittent", "erratic", "lumpy"],
                          p=[0.15, 0.35, 0.10, 0.40])  # taller: domina lumpy/intermitente
    if quadrant == "smooth":
        p_demand, size_mu, size_disp = 0.85, 6.0, 0.25
    elif quadrant == "intermittent":
        p_demand, size_mu, size_disp = 0.35, 4.0, 0.30
    elif quadrant == "erratic":
        p_demand, size_mu, size_disp = 0.80, 8.0, 1.10
    else:  # lumpy
        p_demand, size_mu, size_disp = 0.22, 7.0, 1.20
    occurs = rng.random(WEEKS) < p_demand
    sizes = np.maximum(1, rng.lognormal(np.log(size_mu), size_disp, WEEKS)).round()
    y = np.where(occurs, sizes, 0.0)
    if y[:TRAIN].sum() == 0:  # serie degenerada: fuerza una demanda
        y[rng.integers(0, TRAIN)] = size_mu
    return y, quadrant


def sb_classify(y: np.ndarray) -> str:
    """Clasificación Syntetos-Boylan observada (ADI 1.32, CV2 0.49)."""
    nz = y[y > 0]
    if len(nz) < 2:
        return "lumpy"
    adi = len(y) / max(1, len(nz))
    cv2 = (nz.std(ddof=0) / nz.mean()) ** 2
    if adi < 1.32:
        return "smooth" if cv2 < 0.49 else "erratic"
    return "intermittent" if cv2 < 0.49 else "lumpy"


def croston(y: np.ndarray, alpha: float = ALPHA, variant: str = "croston") -> np.ndarray:
    """Croston clásico / SBA. Devuelve forecast one-step-ahead por periodo."""
    z, p = None, None
    q = 1  # periodos desde la última demanda
    out = np.zeros(len(y))
    for t, yt in enumerate(y):
        out[t] = 0.0 if z is None else z / p
        if yt > 0:
            z = yt if z is None else z + alpha * (yt - z)
            p = q if p is None else p + alpha * (q - p)
            q = 1
        else:
            q += 1
    if variant == "sba":
        out = out * (1 - alpha / 2)
    return out


def tsb(y: np.ndarray, alpha: float = ALPHA, beta: float = BETA) -> np.ndarray:
    """Teunter-Syntetos-Babai: probabilidad de demanda actualizada cada periodo."""
    z, prob = None, 0.5
    out = np.zeros(len(y))
    for t, yt in enumerate(y):
        out[t] = 0.0 if z is None else prob * z
        if yt > 0:
            prob = prob + beta * (1 - prob)
            z = yt if z is None else z + alpha * (yt - z)
        else:
            prob = prob + beta * (0 - prob)
    return out


def moving_mean(y: np.ndarray, w: int = 8) -> np.ndarray:
    out = np.zeros(len(y))
    for t in range(1, len(y)):
        lo = max(0, t - w)
        out[t] = y[lo:t].mean()
    return out


def naive(y: np.ndarray) -> np.ndarray:
    out = np.zeros(len(y))
    out[1:] = y[:-1]
    return out


def insample_mean(y: np.ndarray) -> np.ndarray:
    """Benchmark exigente para demanda intermitente: pronóstico constante a la
    media del tramo de entrenamiento (Hyndman & Koehler-style reference)."""
    out = np.zeros(len(y))
    out[TRAIN:] = y[:TRAIN].mean()
    # dentro de train: media expansiva (evita mirar el futuro)
    for t in range(1, TRAIN):
        out[t] = y[:t].mean()
    return out


METHODS = {
    "croston": lambda y: croston(y, variant="croston"),
    "sba": lambda y: croston(y, variant="sba"),
    "tsb": tsb,
    "mean": insample_mean,
    "naive": naive,
    "ma8": moving_mean,
}


def mase(y: np.ndarray, f: np.ndarray) -> float:
    denom = np.abs(np.diff(y[:TRAIN])).mean()
    if denom == 0:
        return np.nan
    return np.abs(y[TRAIN:] - f[TRAIN:]).mean() / denom


def bias(y: np.ndarray, f: np.ndarray) -> float:
    return float((f[TRAIN:] - y[TRAIN:]).mean())


def main() -> None:
    per_seed: dict[str, dict] = {}
    for seed in SEEDS:
        rng = np.random.default_rng(seed)
        rows = []
        for _ in range(N_SKUS):
            y, _ = gen_sku(rng)
            quad = sb_classify(y[:TRAIN])
            for m, fn in METHODS.items():
                f = fn(y)
                rows.append((quad, m, mase(y, f), bias(y, f)))
        per_seed[str(seed)] = rows

    # agregación: media por (cuadrante, método) por seed → media±std entre seeds
    agg: dict = {"config": {"seeds": SEEDS, "n_skus": N_SKUS, "weeks": WEEKS,
                            "train": TRAIN, "alpha": ALPHA, "beta": BETA},
                 "by_quadrant": {}, "overall": {}}
    quads = ["smooth", "intermittent", "erratic", "lumpy"]
    for quad in quads + ["all"]:
        agg["by_quadrant"][quad] = {}
        for m in METHODS:
            seed_means = []
            seed_bias = []
            for seed in SEEDS:
                vals = [r[2] for r in per_seed[str(seed)]
                        if r[1] == m and (quad == "all" or r[0] == quad) and np.isfinite(r[2])]
                bvals = [r[3] for r in per_seed[str(seed)]
                         if r[1] == m and (quad == "all" or r[0] == quad)]
                if vals:
                    seed_means.append(float(np.mean(vals)))
                    seed_bias.append(float(np.mean(bvals)))
            agg["by_quadrant"][quad][m] = {
                "mase_mean": round(float(np.mean(seed_means)), 3),
                "mase_std": round(float(np.std(seed_means, ddof=1)), 3),
                "bias_mean": round(float(np.mean(seed_bias)), 3),
            }

    out = HERE / "results" / "forecast_results.json"
    out.write_text(json.dumps(agg, indent=2), encoding="utf-8")
    print(f"-> {out}")

    # figura: MASE por cuadrante y método
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    plt.rcParams.update({"font.family": "serif", "font.size": 10,
                         "axes.grid": True, "grid.alpha": 0.3,
                         "figure.figsize": (4.8, 3.0)})
    fig, ax = plt.subplots()
    width = 0.15
    x = np.arange(len(quads))
    labels = {"croston": "CROSTON", "sba": "SBA", "tsb": "TSB",
              "mean": "MEAN", "ma8": "MA(8)", "naive": "NAIVE"}
    methods = ["croston", "sba", "tsb", "mean", "ma8", "naive"]
    width = 0.13
    for i, m in enumerate(methods):
        means = [agg["by_quadrant"][q][m]["mase_mean"] for q in quads]
        stds = [agg["by_quadrant"][q][m]["mase_std"] for q in quads]
        ax.bar(x + (i - 2.5) * width, means, width, yerr=stds, capsize=2, label=labels[m])
    ax.set_xticks(x, [q.capitalize() for q in quads])
    ax.set_ylabel("MASE (test, 26 weeks)")
    ax.legend(ncols=3, fontsize=8)
    fig.tight_layout()
    fig.savefig(HERE / "figures" / "forecast_mase.pdf")
    print("-> figures/forecast_mase.pdf")


if __name__ == "__main__":
    main()
