"""Figura del sitio de despliegue: 2 paneles (muro matricial etiquetado + zona
de almacenamiento general), desde datasets/garage-site/2026-08-10."""
from pathlib import Path

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
from PIL import Image

SRC = Path(r"D:\clarividence\datasets\garage-site\2026-08-10")
HERE = Path(__file__).parent

plt.rcParams.update({"font.family": "serif", "font.size": 9})
fig, axes = plt.subplots(1, 2, figsize=(6.6, 2.6))
for ax, (fname, title) in zip(
    axes,
    [("site-08.jpg", "(a) Matrix-labelled tilt-bin wall (D2/D3-FxxCy)"),
     ("site-01.jpg", "(b) Tilt bins, drawer grids and pegboard, zone T")],
):
    ax.imshow(Image.open(SRC / fname))
    ax.set_title(title, fontsize=8)
    ax.axis("off")
fig.tight_layout()
fig.savefig(HERE / "figures" / "site_overview.pdf", dpi=200, bbox_inches="tight")
print("-> figures/site_overview.pdf")
