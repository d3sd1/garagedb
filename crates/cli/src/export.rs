//! Red de seguridad (spec §7): HTML autocontenido de solo lectura con el
//! BOM, el stock del carro y la checklist, para llevar en el móvil aunque
//! el portátil de circuito muera. Sin scripts externos, sin red.

use garagedb_core::event::Criticality;
use garagedb_core::fold::State;
use garagedb_core::ids::MissionId;
use garagedb_core::mission::{cart_location, mission_check};
use garagedb_core::quantity::Quantity;

pub fn mission_html(state: &State, id: &MissionId) -> anyhow::Result<String> {
    let m = state
        .missions
        .get(id)
        .ok_or_else(|| anyhow::anyhow!("misión desconocida: {id}"))?;
    let kit = state
        .kits
        .get(&m.kit)
        .ok_or_else(|| anyhow::anyhow!("kit desconocido: {}", m.kit))?;
    let cart = cart_location(state, id)?;
    let check = mission_check(state, id)?;

    let crit_label = |c: Criticality| match c {
        Criticality::Blocking => "BLOQUEANTE",
        Criticality::Important => "IMPORTANTE",
        Criticality::Optional => "opcional",
    };

    let mut rows = String::new();
    for line in &kit.lines {
        let have = state
            .stock
            .get(&(line.sku.clone(), cart.clone()))
            .map(|c| match c.qty {
                Quantity::Exact { n } => n.to_string(),
                Quantity::Estimated { n, .. } => format!("~{n}"),
                Quantity::Presence { level } => format!("{level:?}"),
            })
            .unwrap_or_else(|| "0".into());
        rows.push_str(&format!(
            "<tr><td><input type=\"checkbox\"></td><td>{}</td><td>{}</td><td>{}/{}</td><td>{}</td></tr>\n",
            crit_label(line.crit),
            html_escape(line.sku.as_str()),
            line.slot.as_deref().map(html_escape).unwrap_or_default(),
            line.n,
            have,
        ));
    }

    let status = if check.ready {
        "<p class=\"ok\">Sin bloqueantes ausentes en el último check.</p>".to_string()
    } else {
        format!(
            "<p class=\"bad\">FALTAN BLOQUEANTES: {}</p>",
            check
                .missing
                .iter()
                .filter(|l| l.crit == Criticality::Blocking)
                .map(|l| html_escape(l.sku.as_str()))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };

    Ok(format!(
        r#"<!doctype html>
<html lang="es"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>GarageDB — {id} ({circuit})</title>
<style>
 body{{font:20px/1.5 system-ui,sans-serif;margin:16px;background:#fff;color:#111}}
 table{{border-collapse:collapse;width:100%}}
 td,th{{border:1px solid #999;padding:10px;text-align:left;font-size:18px}}
 input[type=checkbox]{{width:28px;height:28px}}
 .ok{{color:#0a6b26;font-weight:700}} .bad{{color:#b00020;font-weight:700}}
 @media print{{ body{{font-size:14px}} }}
</style></head><body>
<h1>Misión {id}</h1>
<p><strong>{circuit}</strong> — {date} — carro {cart} — generado {generated}</p>
{status}
<table>
<tr><th>✓</th><th>Criticidad</th><th>SKU</th><th>Slot/Cant.</th><th>En carro</th></tr>
{rows}
</table>
<p style="color:#666">Documento estático de solo lectura. La fuente de verdad es el almacén GarageDB.</p>
</body></html>
"#,
        id = html_escape(id.as_str()),
        circuit = html_escape(&m.circuit),
        date = html_escape(&m.date),
        cart = html_escape(cart.as_str()),
        generated = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        status = status,
        rows = rows,
    ))
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}
