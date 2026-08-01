//! API HTTP. Política de confirmación (spec §8): un `Count` con
//! `status: confirmed` y `source: ai_vision` se rechaza — la IA solo
//! propone; confirman humano, código de barras o báscula.

use std::sync::Arc;

use axum::extract::{Path as AxPath, Query, State as AxState};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;

use garagedb_core::event::{CountSource, CountStatus, EventBody};
use garagedb_core::ids::MissionId;
use garagedb_core::mission::{
    mission_check, mission_close_events, mission_close_report, try_mark_ready, CloseLine,
    ReadyOutcome,
};
use garagedb_core::quantity::Quantity;

use crate::appstate::AppState;
use crate::ui;

pub fn router(app: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(ui::index))
        .route("/app.js", get(ui::app_js))
        .route("/qr.svg", get(qr_svg))
        .route("/api/health", get(health))
        .route("/api/summary", get(summary))
        .route("/api/stock", get(stock))
        .route("/api/proposals", get(proposals))
        .route("/api/event", post(post_event))
        .route("/api/missions", get(missions))
        .route("/api/mission/:id/check", post(check))
        .route("/api/mission/:id/ready", post(ready))
        .route("/api/mission/:id/close", post(close))
        .route("/api/sync", post(sync))
        .with_state(app)
}

type ApiResult = Result<Response, ApiError>;

struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

impl<E: std::fmt::Display> From<E> for ApiError {
    fn from(e: E) -> Self {
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    }
}

async fn health() -> impl IntoResponse {
    Json(json!({ "ok": true }))
}

async fn summary(AxState(app): AxState<Arc<AppState>>) -> ApiResult {
    Ok(Json(app.summary()?).into_response())
}

#[derive(Deserialize)]
struct StockQuery {
    #[serde(default)]
    q: String,
}

async fn stock(
    AxState(app): AxState<Arc<AppState>>,
    Query(query): Query<StockQuery>,
) -> ApiResult {
    let now_ms = garagedb_core::hlc::wall_now_ms();
    let rows = app.with_state(|s| {
        garagedb_core::search::search(s, &query.q, now_ms, 50)
            .into_iter()
            .map(|hit| {
                let cell = &s.stock[&(hit.sku.clone(), hit.loc.clone())];
                json!({
                    "sku": hit.sku,
                    "name": s.items.get(&hit.sku).map(|i| i.name.clone()).unwrap_or_default(),
                    "unit": s.items.get(&hit.sku).map(|i| i.unit.clone()).unwrap_or_default(),
                    "loc": hit.loc,
                    "qty": cell.qty,
                    "last_verified": cell.last_verified,
                    "stale": cell.stale,
                    "score": (hit.score * 100.0).round() / 100.0,
                })
            })
            .collect::<Vec<_>>()
    })?;
    Ok(Json(rows).into_response())
}

async fn proposals(AxState(app): AxState<Arc<AppState>>) -> ApiResult {
    let rows = app.with_state(|s| {
        s.proposals
            .iter()
            .map(|ev| serde_json::to_value(ev).unwrap_or_default())
            .collect::<Vec<_>>()
    })?;
    Ok(Json(rows).into_response())
}

#[derive(Deserialize)]
struct EventRequest {
    actor: String,
    body: EventBody,
}

async fn post_event(
    AxState(app): AxState<Arc<AppState>>,
    Json(req): Json<EventRequest>,
) -> ApiResult {
    // política de confirmación: la IA no confirma anclas
    if let EventBody::Count { status: CountStatus::Confirmed, source: CountSource::AiVision, .. } =
        &req.body
    {
        return Err(ApiError(
            StatusCode::UNPROCESSABLE_ENTITY,
            "ai_vision no puede confirmar un COUNT: solo human/barcode/scale".into(),
        ));
    }
    // enteros solo: el esquema Quantity ya lo garantiza por construcción
    if let EventBody::Move { delta: 0, .. } = &req.body {
        return Err(ApiError(StatusCode::BAD_REQUEST, "delta 0 no es un movimiento".into()));
    }
    let summary = app.append(&req.actor, req.body)?;
    Ok(Json(summary).into_response())
}

async fn missions(AxState(app): AxState<Arc<AppState>>) -> ApiResult {
    let rows = app.with_state(|s| {
        s.missions
            .iter()
            .map(|(id, m)| {
                json!({
                    "id": id, "date": m.date, "circuit": m.circuit,
                    "kit": m.kit, "vehicle": m.vehicle, "state": m.state,
                })
            })
            .collect::<Vec<_>>()
    })?;
    Ok(Json(rows).into_response())
}

async fn check(
    AxState(app): AxState<Arc<AppState>>,
    AxPath(id): AxPath<String>,
) -> ApiResult {
    let report = app.with_state(|s| mission_check(s, &MissionId::new(id)))?;
    match report {
        Ok(r) => Ok(Json(r).into_response()),
        Err(e) => Err(ApiError(StatusCode::NOT_FOUND, e.to_string())),
    }
}

#[derive(Deserialize)]
struct ActorBody {
    actor: String,
}

async fn ready(
    AxState(app): AxState<Arc<AppState>>,
    AxPath(id): AxPath<String>,
    Json(req): Json<ActorBody>,
) -> ApiResult {
    let mid = MissionId::new(id);
    let outcome = app.with_state(|s| try_mark_ready(s, &mid))?;
    match outcome {
        Ok(ReadyOutcome::Ready(body)) => {
            let summary = app.append(&req.actor, body)?;
            Ok(Json(summary).into_response())
        }
        Ok(ReadyOutcome::Blocked(blockers)) => Ok((
            StatusCode::CONFLICT,
            Json(json!({ "error": "bloqueantes ausentes", "blockers": blockers })),
        )
            .into_response()),
        Err(e) => Err(ApiError(StatusCode::NOT_FOUND, e.to_string())),
    }
}

#[derive(Deserialize)]
struct CloseRequest {
    actor: String,
    lines: Vec<CloseLineDto>,
}

#[derive(Deserialize)]
struct CloseLineDto {
    sku: String,
    n: u64,
    disposition: garagedb_core::event::Disposition,
}

async fn close(
    AxState(app): AxState<Arc<AppState>>,
    AxPath(id): AxPath<String>,
    Json(req): Json<CloseRequest>,
) -> ApiResult {
    let mid = MissionId::new(id);
    let lines: Vec<CloseLine> = req
        .lines
        .into_iter()
        .map(|l| CloseLine {
            sku: garagedb_core::ids::Sku::new(l.sku),
            n: l.n,
            disposition: l.disposition,
        })
        .collect();
    let (events, report) = app.with_state(|s| {
        (
            mission_close_events(s, &mid, &lines),
            mission_close_report(s, &mid, &lines),
        )
    })?;
    match events {
        Ok(evs) => {
            app.append_many(&req.actor, evs)?;
            Ok(Json(report).into_response())
        }
        Err(e) => Err(ApiError(StatusCode::NOT_FOUND, e.to_string())),
    }
}

async fn sync(AxState(app): AxState<Arc<AppState>>) -> ApiResult {
    Ok(Json(app.refold()?).into_response())
}

async fn qr_svg(AxState(app): AxState<Arc<AppState>>) -> ApiResult {
    let _ = &app;
    let url = crate::lan_url();
    let code = qrcode::QrCode::new(url.as_bytes())?;
    let svg = code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(240, 240)
        .build();
    Ok(([("content-type", "image/svg+xml")], svg).into_response())
}

/// Validación de política reutilizada por tests.
pub fn count_confirmed_by_ai(body: &EventBody) -> bool {
    matches!(
        body,
        EventBody::Count { status: CountStatus::Confirmed, source: CountSource::AiVision, .. }
    )
}

/// Helper de construcción de COUNT confirmado por humano (UI/CLI).
pub fn human_count(
    sku: &str,
    loc: &str,
    qty: Quantity,
) -> EventBody {
    EventBody::Count {
        sku: garagedb_core::ids::Sku::new(sku),
        loc: garagedb_core::ids::LocationId::new(loc),
        qty,
        source: CountSource::Human,
        status: CountStatus::Confirmed,
    }
}
