//! Tests de la CLI vía binario (doctor y export-html).

use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_garagedb"))
}

fn run(args: &[&str]) -> (bool, String) {
    let out = bin().args(args).output().expect("ejecutar garagedb");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

#[test]
fn full_flow_init_ingest_mission_export_doctor() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().to_str().unwrap();

    let (ok, _) = run(&["--store", store, "init"]);
    assert!(ok);

    let (ok, _) = run(&["--store", store, "location", "CAR1", "--zone", "G", "--ctype", "mobile_cart", "--mobile"]);
    assert!(ok);
    let (ok, _) = run(&["--store", store, "ingest", "TRANSPONDER", "--loc", "T1-A01", "--n", "1"]);
    assert!(ok);
    let (ok, _) = run(&[
        "--store", store, "kit", "kit1", "--name", "Sprint",
        "--lines", "TRANSPONDER:1:blocking:B1.01,BRIDA-200:8:important",
    ]);
    assert!(ok);
    let (ok, _) = run(&[
        "--store", store, "mission", "create", "m1",
        "--date", "2026-08-02", "--circuit", "Jarama", "--kit", "kit1",
    ]);
    assert!(ok);

    // ready debe fallar: TRANSPONDER no está en el carro
    let (ok, text) = run(&["--store", store, "mission", "ready", "m1"]);
    assert!(!ok, "ready debía fallar con bloqueante ausente");
    assert!(text.contains("BLOQUEANTE"));

    // export html: autocontenido, con los skus, sin scripts externos
    let out_html = dir.path().join("m1.html");
    let (ok, _) = run(&[
        "--store", store, "mission", "export-html", "m1",
        "--out", out_html.to_str().unwrap(),
    ]);
    assert!(ok);
    let html = std::fs::read_to_string(&out_html).unwrap();
    assert!(html.contains("TRANSPONDER"));
    assert!(html.contains("BRIDA-200"));
    assert!(!html.contains("<script src="));

    // doctor sano
    let (ok, text) = run(&["--store", store, "doctor"]);
    assert!(ok, "doctor debía pasar: {text}");
    assert!(text.contains("almacén sano"));
}

#[test]
fn doctor_fails_on_corrupted_signature() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().to_str().unwrap();
    run(&["--store", store, "init"]);
    run(&["--store", store, "ingest", "M6x20", "--loc", "T2-D07", "--n", "10"]);

    // corromper la firma del único evento
    let events_dir = dir.path().join("events");
    let replica_dir = std::fs::read_dir(&events_dir).unwrap().next().unwrap().unwrap().path();
    let shard = std::fs::read_dir(&replica_dir).unwrap().next().unwrap().unwrap().path();
    let content = std::fs::read_to_string(&shard).unwrap();
    let mut ev: serde_json::Value = serde_json::from_str(content.lines().next().unwrap()).unwrap();
    ev["actor"] = serde_json::Value::String("mallory".into());
    std::fs::write(&shard, format!("{ev}\n")).unwrap();

    let (ok, text) = run(&["--store", store, "doctor"]);
    assert!(!ok, "doctor debía fallar con firma corrupta");
    assert!(text.contains("firma"));
}
