//! CLI de GarageDB: interfaz de scripting y diagnóstico.

mod export;

use std::path::PathBuf;

use anyhow::{bail, Context};
use clap::{Parser, Subcommand};

use garagedb_core::event::{
    CountSource, CountStatus, Criticality, Disposition, EventBody, KitLine,
};
use garagedb_core::fold::{fold, state_canonical_json};
use garagedb_core::ids::{KitId, LocationId, MissionId, Sku};
use garagedb_core::mission::{
    mission_check, mission_close_events, mission_close_report, try_mark_ready, CloseLine,
    ReadyOutcome,
};
use garagedb_core::quantity::{FillLevel, Quantity};
use garagedb_core::store::EventStore;

#[derive(Parser)]
#[command(name = "garagedb", version, about = "Inventario de taller sin servidor: log firmado + fold determinista")]
struct Cli {
    /// Ruta del almacén de datos
    #[arg(long, global = true, default_value = ".")]
    store: PathBuf,
    /// Actor que firma la acción (persona)
    #[arg(long, global = true, default_value = "taller")]
    actor: String,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Crea un almacén nuevo (layout + clave de réplica)
    Init,
    /// Arranca el servidor web (LAN + QR)
    Serve {
        #[arg(long, default_value_t = 8080)]
        port: u16,
    },
    /// Identidad de esta réplica
    Whoami,
    /// Rescan de disco + refold (tras git pull / Syncthing / USB)
    Sync,
    /// Regenera state/stock.json canónico
    Refold,
    /// Diagnóstico completo del almacén (exit != 0 si hay errores)
    Doctor,
    /// Alta de material
    Ingest {
        sku: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long, default_value = "general")]
        category: String,
        #[arg(long, default_value = "ud")]
        unit: String,
        #[arg(long)]
        loc: String,
        #[arg(long)]
        n: u64,
    },
    /// Movimiento de stock (consumo negativo, entrada positiva)
    Move {
        sku: String,
        #[arg(long)]
        loc: String,
        #[arg(long, allow_hyphen_values = true)]
        delta: i64,
        #[arg(long, default_value = "ajuste manual")]
        reason: String,
    },
    /// Recuento confirmado (ancla) o nivel presence
    Count {
        sku: String,
        #[arg(long)]
        loc: String,
        #[arg(long, conflicts_with = "level")]
        n: Option<u64>,
        /// full|half|low|empty
        #[arg(long)]
        level: Option<String>,
        /// human|barcode|scale
        #[arg(long, default_value = "human")]
        source: String,
    },
    /// Alta de ubicación
    Location {
        id: String,
        #[arg(long)]
        parent: Option<String>,
        #[arg(long, default_value = "T")]
        zone: String,
        #[arg(long, default_value = "tilt_bin")]
        ctype: String,
        #[arg(long)]
        mobile: bool,
        #[arg(long, value_delimiter = ',')]
        aliases: Vec<String>,
    },
    /// Define un kit: líneas "SKU:n:crit[:slot]" separadas por coma
    Kit {
        id: String,
        #[arg(long)]
        name: String,
        /// ej: TRANSPONDER:1:blocking:B1.01,BRIDA-200:8:important
        #[arg(long, value_delimiter = ',')]
        lines: Vec<String>,
    },
    /// Gestión de misiones
    Mission {
        #[command(subcommand)]
        cmd: MissionCmd,
    },
    /// Consulta de stock
    Stock {
        #[arg(default_value = "")]
        query: String,
    },
}

#[derive(Subcommand)]
enum MissionCmd {
    Create {
        id: String,
        #[arg(long)]
        date: String,
        #[arg(long)]
        circuit: String,
        #[arg(long)]
        kit: String,
        #[arg(long, default_value = "CAR1")]
        vehicle: String,
    },
    /// Diff BOM ↔ carro, con ruta de recogida
    Check { id: String },
    /// Gate de salida: falla con bloqueantes ausentes
    Ready { id: String },
    /// Cierre con reconciliación: líneas "SKU:n:disposition"
    Close {
        id: String,
        /// ej: BRIDA-200:6:consumed,DOT4:1:consumed
        #[arg(long, value_delimiter = ',')]
        lines: Vec<String>,
    },
    /// HTML autocontenido de solo lectura (red de seguridad)
    ExportHtml {
        id: String,
        #[arg(long)]
        out: PathBuf,
    },
}

/// Contexto común (evita el partial-move de `cli.cmd`).
struct Ctx {
    store: PathBuf,
    actor: String,
}

fn main() -> anyhow::Result<()> {
    let parsed = Cli::parse();
    let cli = Ctx { store: parsed.store, actor: parsed.actor };
    match parsed.cmd {
        Cmd::Init => {
            let store = EventStore::init(&cli.store)?;
            println!("Almacén creado en {}", cli.store.display());
            println!("Réplica: {}", store.replica);
            Ok(())
        }
        Cmd::Serve { port } => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(garagedb_server::serve(cli.store, port))
        }
        Cmd::Whoami => {
            let store = EventStore::open(&cli.store)?;
            let report = store.load_all()?;
            let own = report.events.iter().filter(|e| e.replica == store.replica).count();
            println!("Réplica: {}", store.replica);
            println!("Eventos propios: {own} · totales: {}", report.events.len());
            Ok(())
        }
        Cmd::Sync | Cmd::Refold => {
            let store = EventStore::open(&cli.store)?;
            let report = store.load_all()?;
            let state = fold(&report.events);
            store.write_state(&state_canonical_json(&state)?)?;
            println!(
                "{} eventos · {} rechazados · {} líneas malformadas · {} propuestas · {} anomalías",
                report.events.len(),
                report.rejected.len(),
                report.malformed_lines,
                state.proposals.len(),
                state.anomalies.len()
            );
            for r in &report.rejected {
                println!("  RECHAZADO {}:{} — {}", r.file.display(), r.line_no, r.reason);
            }
            Ok(())
        }
        Cmd::Doctor => doctor(&cli.store),
        Cmd::Ingest { sku, name, category, unit, loc, n } => append(
            &cli,
            EventBody::Ingest {
                sku: Sku::new(&sku),
                name: name.unwrap_or_else(|| sku.clone()),
                category,
                unit,
                loc: LocationId::new(loc),
                qty: Quantity::Exact { n },
            },
        ),
        Cmd::Move { sku, loc, delta, reason } => {
            if delta == 0 {
                bail!("delta 0 no es un movimiento");
            }
            append(
                &cli,
                EventBody::Move {
                    sku: Sku::new(sku),
                    loc: LocationId::new(loc),
                    delta,
                    reason,
                    mission: None,
                },
            )
        }
        Cmd::Count { sku, loc, n, level, source } => {
            let qty = match (n, level.as_deref()) {
                (Some(n), None) => Quantity::Exact { n },
                (None, Some(l)) => Quantity::Presence {
                    level: match l {
                        "full" => FillLevel::Full,
                        "half" => FillLevel::Half,
                        "low" => FillLevel::Low,
                        "empty" => FillLevel::Empty,
                        other => bail!("nivel desconocido: {other}"),
                    },
                },
                _ => bail!("indica --n o --level"),
            };
            let source = match source.as_str() {
                "human" => CountSource::Human,
                "barcode" => CountSource::Barcode,
                "scale" => CountSource::Scale,
                // política: ai_vision no confirma; desde CLI ni se acepta
                other => bail!("source inválido para confirmar: {other} (human|barcode|scale)"),
            };
            append(
                &cli,
                EventBody::Count {
                    sku: Sku::new(sku),
                    loc: LocationId::new(loc),
                    qty,
                    source,
                    status: CountStatus::Confirmed,
                },
            )
        }
        Cmd::Location { id, parent, zone, ctype, mobile, aliases } => append(
            &cli,
            EventBody::LocationUpsert {
                id: LocationId::new(id),
                parent: parent.map(LocationId::new),
                zone,
                ctype,
                mobile,
                aliases,
            },
        ),
        Cmd::Kit { id, name, lines } => {
            let parsed: Vec<KitLine> =
                lines.iter().map(|l| parse_kit_line(l)).collect::<anyhow::Result<_>>()?;
            append(&cli, EventBody::KitUpsert { id: KitId::new(id), name, lines: parsed })
        }
        Cmd::Mission { cmd } => mission_cmd(&cli, cmd),
        Cmd::Stock { query } => {
            let store = EventStore::open(&cli.store)?;
            let state = fold(&store.load_all()?.events);
            let q = query.to_lowercase();
            for ((sku, loc), cell) in &state.stock {
                if !q.is_empty()
                    && !sku.as_str().to_lowercase().contains(&q)
                    && !loc.as_str().to_lowercase().contains(&q)
                {
                    continue;
                }
                let qty = match cell.qty {
                    Quantity::Exact { n } => n.to_string(),
                    Quantity::Estimated { n, .. } => format!("~{n}"),
                    Quantity::Presence { level } => format!("{level:?}"),
                };
                let stale = if cell.stale { " [REVISAR]" } else { "" };
                println!("{sku:<24} {loc:<14} {qty:>8}{stale}");
            }
            Ok(())
        }
    }
}

fn append(cli: &Ctx, body: EventBody) -> anyhow::Result<()> {
    let mut store = EventStore::open(&cli.store)?;
    let ev = store.append(&cli.actor, body)?;
    // mantener el derivado al día
    let state = fold(&store.load_all()?.events);
    store.write_state(&state_canonical_json(&state)?)?;
    println!("OK {} (seq {})", ev.id, ev.seq);
    Ok(())
}

fn parse_kit_line(s: &str) -> anyhow::Result<KitLine> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() < 3 {
        bail!("línea de kit inválida: {s} (formato SKU:n:crit[:slot])");
    }
    Ok(KitLine {
        sku: Sku::new(parts[0]),
        n: parts[1].parse().context("cantidad inválida")?,
        crit: match parts[2] {
            "blocking" => Criticality::Blocking,
            "important" => Criticality::Important,
            "optional" => Criticality::Optional,
            other => bail!("criticidad desconocida: {other}"),
        },
        slot: parts.get(3).map(|s| s.to_string()),
    })
}

fn parse_close_line(s: &str) -> anyhow::Result<CloseLine> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 3 {
        bail!("línea de cierre inválida: {s} (formato SKU:n:disposition)");
    }
    Ok(CloseLine {
        sku: Sku::new(parts[0]),
        n: parts[1].parse().context("cantidad inválida")?,
        disposition: match parts[2] {
            "consumed" => Disposition::Consumed,
            "broken" => Disposition::Broken,
            "lost" => Disposition::Lost,
            "lent" => Disposition::Lent,
            "misplaced" => Disposition::Misplaced,
            "returned" => Disposition::Returned,
            other => bail!("disposición desconocida: {other}"),
        },
    })
}

fn mission_cmd(cli: &Ctx, cmd: MissionCmd) -> anyhow::Result<()> {
    match cmd {
        MissionCmd::Create { id, date, circuit, kit, vehicle } => append(
            cli,
            EventBody::MissionCreate {
                id: MissionId::new(id),
                date,
                circuit,
                kit: KitId::new(kit),
                vehicle,
            },
        ),
        MissionCmd::Check { id } => {
            let store = EventStore::open(&cli.store)?;
            let state = fold(&store.load_all()?.events);
            let report = mission_check(&state, &MissionId::new(id))?;
            if report.missing.is_empty() {
                println!("✔ Carro completo.");
            }
            for l in &report.missing {
                println!(
                    "{:?} {} — hay {} de {} · coger de: {}",
                    l.crit,
                    l.sku,
                    l.have,
                    l.have + l.need,
                    l.sources
                        .iter()
                        .map(|(loc, n)| format!("{loc} ({n})"))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            println!("\nRuta de recogida:");
            for (loc, items) in &report.route {
                println!("  {loc}:");
                for (sku, n) in items {
                    println!("    ☐ {n}× {sku}");
                }
            }
            println!("\n{}", if report.ready { "LISTO (sin bloqueantes)" } else { "NO LISTO" });
            Ok(())
        }
        MissionCmd::Ready { id } => {
            let mid = MissionId::new(id);
            let store = EventStore::open(&cli.store)?;
            let state = fold(&store.load_all()?.events);
            match try_mark_ready(&state, &mid)? {
                ReadyOutcome::Ready(body) => append(cli, body),
                ReadyOutcome::Blocked(blockers) => {
                    for b in &blockers {
                        println!("⛔ BLOQUEANTE ausente: {} (faltan {})", b.sku, b.need);
                    }
                    bail!("misión NO lista: {} bloqueante(s)", blockers.len());
                }
            }
        }
        MissionCmd::Close { id, lines } => {
            let mid = MissionId::new(id);
            let parsed: Vec<CloseLine> =
                lines.iter().map(|l| parse_close_line(l)).collect::<anyhow::Result<_>>()?;
            let store = EventStore::open(&cli.store)?;
            let state = fold(&store.load_all()?.events);
            let events = mission_close_events(&state, &mid, &parsed)?;
            let report = mission_close_report(&state, &mid, &parsed);
            let mut store = EventStore::open(&cli.store)?;
            for body in events {
                store.append(&cli.actor, body)?;
            }
            let state = fold(&store.load_all()?.events);
            store.write_state(&state_canonical_json(&state)?)?;
            println!("Misión cerrada.");
            for (sku, n) in &report.consumed {
                println!("  consumido: {n}× {sku}");
            }
            for (sku, n, sources) in &report.restock_suggestions {
                let src = sources
                    .iter()
                    .map(|(l, k)| format!("{l} ({k})"))
                    .collect::<Vec<_>>()
                    .join(", ");
                println!("  reponer {n}× {sku} — desde: {}", if src.is_empty() { "COMPRAR (sin stock estático)" } else { &src });
            }
            Ok(())
        }
        MissionCmd::ExportHtml { id, out } => {
            let store = EventStore::open(&cli.store)?;
            let state = fold(&store.load_all()?.events);
            let html = export::mission_html(&state, &MissionId::new(id))?;
            std::fs::write(&out, html)?;
            println!("Exportado: {}", out.display());
            Ok(())
        }
    }
}

fn doctor(root: &PathBuf) -> anyhow::Result<()> {
    let store = EventStore::open(root)?;
    let report = store.load_all()?;
    let state = fold(&report.events);
    let mut errors = 0u32;

    println!("GarageDB doctor — {}", root.display());
    println!("Réplica local: {}", store.replica);
    println!("Eventos válidos: {}", report.events.len());

    if report.malformed_lines > 0 {
        errors += report.malformed_lines;
        println!("✗ {} línea(s) malformada(s) en shards (escritura interrumpida)", report.malformed_lines);
    }
    for r in &report.rejected {
        errors += 1;
        println!("✗ RECHAZADO {}:{} — {}", r.file.display(), r.line_no, r.reason);
    }
    // divergencia state/ vs fold
    let expected = state_canonical_json(&state)?;
    let state_path = root.join("state/stock.json");
    match std::fs::read_to_string(&state_path) {
        Ok(on_disk) if on_disk == expected => println!("✔ state/ coincide con el fold del log"),
        Ok(_) => {
            errors += 1;
            println!("✗ state/stock.json diverge del log — ejecuta `garagedb refold`");
        }
        Err(_) => println!("· state/stock.json no existe aún (se genera con refold)"),
    }
    // árbol de ubicaciones: padres existentes
    for (id, meta) in &state.locations {
        for p in meta.parent.iter().chain(meta.current_parent.iter()) {
            if !state.locations.contains_key(p) && p.as_str().len() > 1 {
                println!("· aviso: {id} apunta a padre no declarado {p}");
            }
        }
    }
    if state.anomalies.is_empty() {
        println!("✔ sin anomalías de fold");
    } else {
        println!("· {} anomalía(s) de fold (informativas):", state.anomalies.len());
        for a in state.anomalies.iter().take(10) {
            println!("    {a}");
        }
    }
    if errors > 0 {
        bail!("doctor: {errors} error(es)");
    }
    println!("✔ almacén sano");
    Ok(())
}
