"""Agregador de los benches de comparación de almacenes (faltaba en el
paquete: review R2/R3-N16a). Entrada: results/{stores_raw,automerge_raw}.jsonl.
Salida: results/store_comparison.json con Automerge por-dirección y bytes."""
import json
import statistics as st
from pathlib import Path

HERE = Path(__file__).parent
R = HERE / "results"


def agg(path, keys):
    rows = [json.loads(l) for l in open(path, encoding="utf-8")]
    out = {}
    for n in sorted({r["n"] for r in rows}):
        g = [r for r in rows if r["n"] == n]
        out[str(n)] = {
            k: {"mean": round(st.mean(r[k] for r in g), 1),
                "std": round(st.stdev(r[k] for r in g), 1)}
            for k in keys
        }
    return out


stores = agg(R / "stores_raw.jsonl", ["garagedb_ms", "sqlite_insert_ms", "sqlite_fold_ms"])
am = agg(R / "automerge_raw.jsonl", ["am_build_ms", "am_merge_both_ms", "am_saved_bytes"])

comparison = {"stores": stores, "automerge": {}}
for n, v in am.items():
    both = v["am_merge_both_ms"]
    comparison["automerge"][n] = {
        **v,
        # el bench mide AMBAS direcciones (2 saves + 4 loads + 2 merges);
        # por-dirección = mitad, la magnitud comparable a una convergencia
        "am_merge_per_direction_ms": {"mean": round(both["mean"] / 2, 1),
                                      "std": round(both["std"] / 2, 1)},
        "am_bytes_per_event": round(v["am_saved_bytes"]["mean"] / int(n), 1),
    }

(R / "store_comparison.json").write_text(json.dumps(comparison, indent=2), encoding="utf-8")
for n in stores:
    g = stores[n]["garagedb_ms"]["mean"]
    si = stores[n]["sqlite_insert_ms"]["mean"]
    sf = stores[n]["sqlite_fold_ms"]["mean"]
    ad = comparison["automerge"][n]["am_merge_per_direction_ms"]["mean"]
    print(f"n={n}: gdb {g} ms | sqlite {si}+{sf} (ratio {round(g/(si+sf),1)}x) | "
          f"automerge/dir {ad} ms | am B/ev {comparison['automerge'][n]['am_bytes_per_event']}")
