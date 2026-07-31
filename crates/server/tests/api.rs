//! Tests de la API (tower::oneshot, sin red).

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use garagedb_server::api::router;
use garagedb_server::appstate::AppState;

async fn body_json(res: axum::response::Response) -> serde_json::Value {
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

fn post(uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

async fn app() -> (Arc<AppState>, axum::Router, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::open(dir.path()).unwrap();
    let r = router(state.clone());
    (state, r, dir)
}

#[tokio::test]
async fn move_event_reflects_in_stock() {
    let (_s, r, _d) = app().await;
    let res = r
        .clone()
        .oneshot(post(
            "/api/event",
            serde_json::json!({
                "actor": "test",
                "body": {"op":"count","sku":"M6x20","loc":"T2-D07",
                         "qty":{"kind":"exact","n":10},"source":"human","status":"confirmed"}
            }),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let res = r
        .clone()
        .oneshot(post(
            "/api/event",
            serde_json::json!({
                "actor": "test",
                "body": {"op":"move","sku":"M6x20","loc":"T2-D07","delta":-4,
                         "reason":"montaje","mission":null}
            }),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let res = r.clone().oneshot(get("/api/stock?q=m6")).await.unwrap();
    let rows = body_json(res).await;
    assert_eq!(rows[0]["qty"]["n"], 6);
}

#[tokio::test]
async fn ai_vision_cannot_confirm_count() {
    let (_s, r, _d) = app().await;
    let res = r
        .clone()
        .oneshot(post(
            "/api/event",
            serde_json::json!({
                "actor": "vision-worker",
                "body": {"op":"count","sku":"M6x20","loc":"T2-D07",
                         "qty":{"kind":"exact","n":42},"source":"ai_vision","status":"confirmed"}
            }),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // como proposed sí entra, y queda en la cola
    let res = r
        .clone()
        .oneshot(post(
            "/api/event",
            serde_json::json!({
                "actor": "vision-worker",
                "body": {"op":"count","sku":"M6x20","loc":"T2-D07",
                         "qty":{"kind":"exact","n":42},"source":"ai_vision","status":"proposed"}
            }),
        ))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let res = r.clone().oneshot(get("/api/proposals")).await.unwrap();
    let props = body_json(res).await;
    assert_eq!(props.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn ready_blocked_by_missing_blocking_item() {
    let (_s, r, _d) = app().await;
    for body in [
        serde_json::json!({"op":"location_upsert","id":"CAR1","parent":"G","zone":"G",
                           "ctype":"mobile_cart","mobile":true,"aliases":[]}),
        serde_json::json!({"op":"kit_upsert","id":"kit1","name":"Sprint","lines":[
            {"sku":"TRANSPONDER","n":1,"crit":"blocking","slot":null}]}),
        serde_json::json!({"op":"mission_create","id":"m1","date":"2026-08-02",
                           "circuit":"Jarama","kit":"kit1","vehicle":"CAR1"}),
    ] {
        let res = r
            .clone()
            .oneshot(post("/api/event", serde_json::json!({"actor":"t","body":body})))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }
    let res = r
        .clone()
        .oneshot(post("/api/mission/m1/ready", serde_json::json!({"actor":"t"})))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CONFLICT);
    let body = body_json(res).await;
    assert_eq!(body["blockers"][0]["sku"], "TRANSPONDER");
}

#[tokio::test]
async fn sync_incorporates_externally_copied_shard() {
    let (state, r, dir) = app().await;
    // segunda réplica en otro dir, genera actividad, y "Syncthing" copia
    let dir_b = tempfile::tempdir().unwrap();
    let mut b = garagedb_core::store::EventStore::init(dir_b.path()).unwrap();
    b.append(
        "remoto",
        garagedb_core::event::EventBody::Count {
            sku: garagedb_core::ids::Sku::new("DOT4"),
            loc: garagedb_core::ids::LocationId::new("A1-N1-P1"),
            qty: garagedb_core::quantity::Quantity::Exact { n: 3 },
            source: garagedb_core::event::CountSource::Human,
            status: garagedb_core::event::CountStatus::Confirmed,
        },
    )
    .unwrap();
    // copiar events/ y config/replicas/ de B a A
    for sub in ["events", "config/replicas"] {
        let from = dir_b.path().join(sub);
        let to = dir.path().join(sub);
        copy_tree(&from, &to);
    }
    let res = r.clone().oneshot(post("/api/sync", serde_json::json!({}))).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let s = body_json(res).await;
    assert_eq!(s["n_events"], 1);
    let n = state.with_state(|st| st.stock.len()).unwrap();
    assert_eq!(n, 1);
}

fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &dest);
        } else {
            std::fs::copy(entry.path(), &dest).unwrap();
        }
    }
}
