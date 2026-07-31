# GarageDB

Inventario físico para taller de competición, **sin servidor, sin nube, sin base de datos**: el estado vive en un almacén de ficheros sincronizable por Git, Syncthing, una carpeta de red o un USB. Con IA local opcional (fases posteriores).

- **Log de eventos append-only firmado (ed25519)**, particionado por réplica: dos máquinas jamás escriben el mismo fichero, la fusión es unión de ficheros y **no existen conflictos**.
- **Fold determinista** con orden total (HLC): el mismo conjunto de eventos produce byte a byte el mismo estado en cualquier máquina.
- **Misiones / carro de circuito**: kits con criticidad (`blocking` impide salir sin el material), checklist por ruta física de recogida, reconciliación al volver.
- **UI móvil** servida en LAN por el propio binario (QR en consola) + **CLI completa**.

## Quickstart

```bash
cargo build --release
./target/release/garagedb --store D:/garaje-data init
./target/release/garagedb --store D:/garaje-data serve
# escanea el QR con el móvil → UI en el navegador
```

### Flujo típico

```bash
garagedb --store D:/garaje-data location CAR1 --zone G --ctype mobile_cart --mobile
garagedb --store D:/garaje-data ingest M6x20-DIN912 --loc T2-D07 --n 100
garagedb --store D:/garaje-data kit kit-sprint --name "Sprint" \
  --lines "TRANSPONDER:1:blocking:B1.01,BRIDA-200:8:important"
garagedb --store D:/garaje-data mission create jarama-0802 \
  --date 2026-08-02 --circuit Jarama --kit kit-sprint
garagedb --store D:/garaje-data mission check jarama-0802     # qué falta, por ruta
garagedb --store D:/garaje-data mission ready jarama-0802     # falla si falta un blocking
garagedb --store D:/garaje-data mission export-html jarama-0802 --out carro.html  # red de seguridad
garagedb --store D:/garaje-data mission close jarama-0802 --lines "BRIDA-200:6:consumed"
```

### Sincronización (transporte `folder`)

El almacén es una carpeta. Sincronízala como quieras:

- **Git**: `git init` en el almacén, remoto donde quieras. Tras `git pull`: `garagedb sync`.
- **Syncthing**: comparte la carpeta. `garagedb sync` (o botón Sync de la UI) incorpora lo nuevo.
- **USB**: copia `events/` y `config/replicas/` de una máquina a otra. `garagedb sync`.

`state/` y `.local/` (clave privada) quedan fuera de sincronización — ya lo excluyen `.gitignore`/`.stignore` generados por `init`.

### Diagnóstico

```bash
garagedb --store D:/garaje-data doctor   # firmas, integridad, divergencia state/log
```

## Estado del proyecto (roadmap por fases)

| Fase | Estado |
|---|---|
| 0-3: núcleo firmado + folder + misiones/carro + UI/CLI | ✅ v0.1 |
| 4: transporte `git` integrado | pendiente |
| 5: consumibles/caducidades + chat planificador | pendiente |
| 6: instaladores (Tauri) | pendiente |
| 7: transporte `p2p` (iroh) | pendiente |
| 8-10: herramienta/shadow boards, ingesta por visión, mapa 2D | pendiente |

Diseño completo y decisiones: ver spec del proyecto (repo de investigación).
