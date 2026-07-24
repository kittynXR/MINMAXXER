use crate::config::{AppConfig, OVERLAY_SETTINGS_SCHEMA_VERSION};
use crate::storage::Storage;
use crate::tailer::{CollectorHandle, CollectorSettings};
use crate::vr_overlay::VrOverlayStatus;
use axum::extract::{DefaultBodyLimit, Multipart, Path as AxumPath, Query, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{Event as SseEvent, KeepAlive};
use axum::response::{IntoResponse, Response, Sse};
use axum::routing::{get, post};
use axum::{Json, Router};
use minmaxxer_core::{analyze_runs, EngineSnapshot};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::convert::Infallible;
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio_stream::wrappers::WatchStream;
use tokio_stream::StreamExt;

const INDEX_HTML: &str = include_str!("../../../ui/index.html");
const OVERLAY_HTML: &str = include_str!("../../../ui/overlay/index.html");
const APP_JS: &str = include_str!("../../../ui/app.js");
const STYLE_CSS: &str = include_str!("../../../ui/style.css");
const MUTATION_TOKEN_HEADER: &str = "x-minmaxxer-token";

#[derive(Clone)]
pub struct ServerState {
    pub snapshots: watch::Receiver<EngineSnapshot>,
    pub storage: Arc<Storage>,
    pub config: Arc<RwLock<AppConfig>>,
    pub config_path: PathBuf,
    pub config_updates: watch::Sender<AppConfig>,
    pub collector: CollectorHandle,
    pub imports_directory: PathBuf,
    pub started_at: Instant,
    pub vr_status: Option<watch::Receiver<VrOverlayStatus>>,
}

pub async fn serve_on(state: ServerState, listener: std::net::TcpListener) -> anyhow::Result<()> {
    let port = listener.local_addr()?.port();
    let app = Router::new()
        .route("/", get(index))
        .route("/index.html", get(index))
        .route("/overlay", get(overlay))
        .route("/overlay/", get(overlay))
        .route("/overlay/index.html", get(overlay))
        .route("/style.css", get(styles))
        .route("/app.js", get(script))
        .route("/api/health", get(health))
        .route("/api/live", get(live))
        .route("/api/stream", get(stream))
        .route("/api/runs", get(runs))
        .route("/api/runs/{id}", get(run_detail))
        .route("/api/events", get(events))
        .route("/api/settings", get(settings).put(update_settings))
        .route("/api/vr-status", get(vr_status))
        .route("/api/import", post(import_log))
        .route("/api/rescan", post(rescan))
        .layer(DefaultBodyLimit::max(96 * 1024 * 1024))
        .layer(middleware::from_fn(require_loopback_host))
        .with_state(state);
    let listener = TcpListener::from_std(listener)?;
    tracing::info!(url = %format!("http://127.0.0.1:{port}"), "local HUD server listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn require_loopback_host(request: Request, next: Next) -> Response {
    let allowed = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .is_some_and(is_allowed_host);
    if !allowed {
        return error_response(
            StatusCode::FORBIDDEN,
            "invalid local Host header".to_owned(),
        );
    }
    next.run(request).await
}

fn is_allowed_host(host: &str) -> bool {
    let hostname = host
        .rsplit_once(':')
        .map(|(hostname, _)| hostname)
        .unwrap_or(host);
    matches!(
        hostname.to_ascii_lowercase().as_str(),
        "127.0.0.1" | "localhost"
    )
}

async fn index() -> Response {
    static_response(INDEX_HTML, "text/html; charset=utf-8")
}

async fn overlay() -> Response {
    static_response(OVERLAY_HTML, "text/html; charset=utf-8")
}

async fn styles() -> Response {
    static_response(STYLE_CSS, "text/css; charset=utf-8")
}

async fn script() -> Response {
    static_response(APP_JS, "text/javascript; charset=utf-8")
}

fn static_response(body: &'static str, content_type: &'static str) -> Response {
    let mut response = body.into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; connect-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; script-src 'self'; frame-ancestors 'none'",
        ),
    );
    response.headers_mut().insert(
        header::HeaderName::from_static("x-frame-options"),
        HeaderValue::from_static("DENY"),
    );
    response
}

async fn health(State(state): State<ServerState>) -> Response {
    match state.storage.stats() {
        Ok(storage) => Json(json!({
            "ok": true,
            "version": env!("CARGO_PKG_VERSION"),
            "uptime_seconds": state.started_at.elapsed().as_secs(),
            "storage": storage,
            "api": 2,
        }))
        .into_response(),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

async fn live(State(state): State<ServerState>) -> Json<EngineSnapshot> {
    Json(state.snapshots.borrow().clone())
}

async fn vr_status(State(state): State<ServerState>) -> Json<VrOverlayStatus> {
    Json(
        state
            .vr_status
            .as_ref()
            .map(|status| status.borrow().clone())
            .unwrap_or_default(),
    )
}

async fn stream(
    State(state): State<ServerState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<SseEvent, Infallible>>> {
    let stream = WatchStream::new(state.snapshots.clone()).map(|snapshot| {
        let data = serde_json::to_string(&snapshot).unwrap_or_else(|_| "{}".to_owned());
        Ok(SseEvent::default().event("message").data(data))
    });
    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(10))
            .text("keep-alive"),
    )
}

async fn runs(State(state): State<ServerState>) -> Response {
    let storage = state.storage.clone();
    match tokio::task::spawn_blocking(move || {
        storage.all_events().map(|events| analyze_runs(&events))
    })
    .await
    {
        Ok(Ok(runs)) => Json(json!({ "runs": runs })).into_response(),
        Ok(Err(error)) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

async fn run_detail(State(state): State<ServerState>, AxumPath(id): AxumPath<String>) -> Response {
    let storage = state.storage.clone();
    let query_id = id.clone();
    match tokio::task::spawn_blocking(move || {
        storage.events_for_run(&query_id).map(|events| {
            let run = analyze_runs(&events)
                .into_iter()
                .find(|run| run.id == query_id);
            (run, events)
        })
    })
    .await
    {
        Ok(Ok((Some(run), events))) => {
            Json(json!({ "run": run, "events": events })).into_response()
        }
        Ok(Ok((None, _))) => {
            error_response(StatusCode::NOT_FOUND, format!("run {id} was not found"))
        }
        Ok(Err(error)) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

#[derive(Debug, Deserialize)]
struct EventQuery {
    run_id: Option<String>,
    limit: Option<usize>,
}

async fn events(State(state): State<ServerState>, Query(query): Query<EventQuery>) -> Response {
    let storage = state.storage.clone();
    let EventQuery { run_id, limit } = query;
    match tokio::task::spawn_blocking(move || match run_id {
        Some(run_id) => storage.events_for_run(&run_id),
        None => storage.all_events(),
    })
    .await
    {
        Ok(Ok(mut events)) => {
            if let Some(limit) = limit {
                let keep_from = events.len().saturating_sub(limit.min(10_000));
                events.drain(..keep_from);
            }
            Json(json!({ "events": events })).into_response()
        }
        Ok(Err(error)) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

async fn settings(State(state): State<ServerState>) -> Response {
    let config = state.config.read().expect("config lock poisoned").clone();
    let mut value = serde_json::to_value(&config).unwrap_or_else(|_| json!({}));
    if let Value::Object(root) = &mut value {
        root.insert(
            "log_path".to_owned(),
            Value::String(config.log_directory.to_string_lossy().into_owned()),
        );
        root.insert(
            "desktop_overlay_enabled".to_owned(),
            Value::Bool(config.desktop_overlay.enabled),
        );
        root.insert(
            "vr_overlay_enabled".to_owned(),
            Value::Bool(config.vr_overlay.enabled),
        );
        root.insert(
            "obs_url".to_owned(),
            Value::String(format!("http://127.0.0.1:{}/overlay", config.port)),
        );
    }
    Json(value).into_response()
}

async fn update_settings(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Json(patch): Json<Value>,
) -> Response {
    if let Some(response) = reject_unauthorized_mutation(&state, &headers) {
        return response;
    }
    let current = state.config.read().expect("config lock poisoned").clone();
    let mut merged = serde_json::to_value(&current).unwrap_or_else(|_| json!({}));
    apply_compatibility_aliases(&mut merged, &patch);
    deep_merge(&mut merged, patch);
    merged["overlay_schema_version"] = json!(OVERLAY_SETTINGS_SCHEMA_VERSION);
    let next: AppConfig = match serde_json::from_value(merged) {
        Ok(config) => config,
        Err(error) => return error_response(StatusCode::BAD_REQUEST, error.to_string()),
    };
    if let Err(error) = next.validate() {
        return error_response(StatusCode::BAD_REQUEST, error.to_string());
    }
    if next.port != current.port {
        return error_response(
            StatusCode::BAD_REQUEST,
            format!(
                "port changes require an app restart; the running server remains bound to {}",
                current.port
            ),
        );
    }
    if let Err(error) = next.save(&state.config_path) {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string());
    }
    *state.config.write().expect("config lock poisoned") = next.clone();
    if let Err(error) = state.collector.reconfigure(CollectorSettings::from(&next)) {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string());
    }
    let _ = state.config_updates.send(next.clone());
    Json(json!({
        "ok": true,
        "desktop_overlay_enabled": next.desktop_overlay.enabled,
        "vr_overlay_enabled": next.vr_overlay.enabled,
    }))
    .into_response()
}

async fn import_log(
    State(state): State<ServerState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Response {
    if let Some(response) = reject_unauthorized_mutation(&state, &headers) {
        return response;
    }
    if let Err(error) = std::fs::create_dir_all(&state.imports_directory) {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string());
    }
    let mut imported_paths = Vec::new();
    let mut unique_paths = HashSet::new();
    let monitored_directory = state
        .config
        .read()
        .expect("config lock poisoned")
        .log_directory
        .clone();
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(error) => return error_response(StatusCode::BAD_REQUEST, error.to_string()),
        };
        if field.name() != Some("file") {
            continue;
        }
        let bytes = match field.bytes().await {
            Ok(bytes) => bytes,
            Err(error) => return error_response(StatusCode::BAD_REQUEST, error.to_string()),
        };
        let fingerprint = content_fingerprint(&bytes);
        let monitored_match = match find_monitored_duplicate(
            &monitored_directory,
            bytes.len() as u64,
            &fingerprint,
        ) {
            Ok(path) => path,
            Err(error) => {
                tracing::warn!(%error, "could not check monitored logs for an identical import");
                None
            }
        };
        let path = match monitored_match
            .map(Ok)
            .unwrap_or_else(|| persist_uploaded_log(&state.imports_directory, &bytes, &fingerprint))
        {
            Ok(path) => path,
            Err(error) => {
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
            }
        };
        if unique_paths.insert(path.clone()) {
            imported_paths.push(path);
        }
    }
    if imported_paths.is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "no log file was uploaded".to_owned(),
        );
    }
    let collector = state.collector.clone();
    let count = imported_paths.len();
    match tokio::task::spawn_blocking(move || {
        for path in imported_paths {
            collector.import(path)?;
        }
        anyhow::Ok(())
    })
    .await
    {
        Ok(Ok(())) => Json(json!({
            "ok": true,
            "message": format!("Imported {count} log file{} and merged matching session data.", if count == 1 { "" } else { "s" })
        }))
        .into_response(),
        Ok(Err(error)) => error_response(StatusCode::BAD_REQUEST, error.to_string()),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

async fn rescan(State(state): State<ServerState>, headers: HeaderMap) -> Response {
    if let Some(response) = reject_unauthorized_mutation(&state, &headers) {
        return response;
    }
    match state.collector.rescan() {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(error) => error_response(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

fn reject_unauthorized_mutation(state: &ServerState, headers: &HeaderMap) -> Option<Response> {
    let expected = state
        .config
        .read()
        .expect("config lock poisoned")
        .stream_token
        .clone();
    let supplied = headers
        .get(MUTATION_TOKEN_HEADER)
        .and_then(|value| value.to_str().ok());
    (supplied != Some(expected.as_str())).then(|| {
        error_response(
            StatusCode::FORBIDDEN,
            "missing or invalid local mutation token".to_owned(),
        )
    })
}

fn apply_compatibility_aliases(base: &mut Value, patch: &Value) {
    let Some(patch) = patch.as_object() else {
        return;
    };
    base["overlay_schema_version"] = json!(OVERLAY_SETTINGS_SCHEMA_VERSION);
    if let Some(value) = patch
        .get("desktop_overlay_enabled")
        .and_then(Value::as_bool)
    {
        base["desktop_overlay"]["enabled"] = Value::Bool(value);
    }
    if let Some(value) = patch.get("vr_overlay_enabled").and_then(Value::as_bool) {
        base["vr_overlay"]["enabled"] = Value::Bool(value);
    }
    if let Some(value) = patch
        .get("log_path")
        .or_else(|| patch.get("logPath"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    {
        base["log_directory"] = Value::String(expand_profile(value));
    }
    if let Some(overlay) = patch.get("overlay") {
        let overlay_object = overlay.as_object();
        if let Some(object) = overlay_object {
            let requested_profile = object
                .get("profile")
                .and_then(Value::as_str)
                .filter(|profile| matches!(*profile, "broadcast" | "minimal" | "vr"))
                .unwrap_or("broadcast")
                .to_owned();
            let profile_index = base
                .get_mut("overlay_profiles")
                .and_then(Value::as_array_mut)
                .map(|profiles| {
                    if let Some(index) = profiles.iter().position(|profile| {
                        profile.get("id").and_then(Value::as_str)
                            == Some(requested_profile.as_str())
                    }) {
                        return index;
                    }
                    let mut profile = profiles.first().cloned().unwrap_or_else(|| json!({}));
                    profile["id"] = Value::String(requested_profile.clone());
                    profile["name"] = Value::String(
                        match requested_profile.as_str() {
                            "minimal" => "Minimal HUD",
                            "vr" => "VR HUD",
                            _ => "Broadcast HUD",
                        }
                        .to_owned(),
                    );
                    profiles.push(profile);
                    profiles.len() - 1
                });
            base["desktop_overlay"]["profile"] = Value::String(requested_profile);

            if let Some(profile) = profile_index.and_then(|index| {
                base.get_mut("overlay_profiles")
                    .and_then(Value::as_array_mut)
                    .and_then(|profiles| profiles.get_mut(index))
            }) {
                for key in ["layout", "theme", "accent", "rows"] {
                    if let Some(value) = object.get(key) {
                        profile[key] = value.clone();
                    }
                }
                if let Some(scale) = object.get("scale").and_then(Value::as_f64) {
                    profile["scale"] = json!(if scale > 10.0 { scale / 100.0 } else { scale });
                }
                if let Some(rows) = object
                    .get("hit_rows")
                    .or_else(|| object.get("hitRows"))
                    .and_then(Value::as_u64)
                {
                    profile["recent_hit_rows"] = json!(rows.clamp(1, 12));
                }
                if let Some(show) = object.get("show").and_then(Value::as_array) {
                    let has = |name: &str| show.iter().any(|value| value.as_str() == Some(name));
                    let current_schema = object
                        .get("schema_version")
                        .or_else(|| object.get("ui"))
                        .and_then(Value::as_u64)
                        .is_some_and(|version| {
                            version >= u64::from(OVERLAY_SETTINGS_SCHEMA_VERSION)
                        });
                    let legacy_run_context = !current_schema && has("encounter");
                    profile["show_dps"] = Value::Bool(has("dps"));
                    profile["show_damage"] = Value::Bool(has("damage"));
                    profile["show_incoming"] = Value::Bool(has("incoming"));
                    profile["show_hits"] = Value::Bool(has("hits"));
                    profile["show_recent_hits"] = Value::Bool(has("recent_hits") || has("hits"));
                    profile["show_encounter"] = Value::Bool(has("encounter"));
                    profile["show_phase"] = Value::Bool(has("phase") || legacy_run_context);
                    profile["show_boss_number"] = Value::Bool(has("boss") || legacy_run_context);
                    profile["show_focus"] = Value::Bool(has("focus"));
                    profile["show_graph"] = Value::Bool(has("graph"));
                    profile["show_survival"] = Value::Bool(has("survival"));
                    profile["show_telemetry"] = Value::Bool(current_schema && has("telemetry"));
                    profile["show_loadout"] = Value::Bool(has("loadout"));
                }
            }

            if let Some(background) = object.get("bg").and_then(Value::as_f64) {
                let opacity = if background > 1.0 {
                    background / 100.0
                } else {
                    background
                }
                .clamp(0.0, 1.0);
                base["desktop_overlay"]["opacity"] = json!(opacity);
            }

            // The browser/desktop surfaces consume OverlayProfile. Mirror the same content
            // choices into the native renderer so Studio configures every output it can express.
            if let Some(rows) = object.get("rows").and_then(Value::as_u64) {
                base["vr_overlay"]["rows"] = json!(rows.clamp(1, 8));
            }
            if let Some(rows) = object
                .get("hit_rows")
                .or_else(|| object.get("hitRows"))
                .and_then(Value::as_u64)
            {
                base["vr_overlay"]["recent_hit_rows"] = json!(rows.clamp(1, 8));
            }
            if let Some(show) = object.get("show").and_then(Value::as_array) {
                let has = |name: &str| show.iter().any(|value| value.as_str() == Some(name));
                let current_schema = object
                    .get("schema_version")
                    .or_else(|| object.get("ui"))
                    .and_then(Value::as_u64)
                    .is_some_and(|version| version >= u64::from(OVERLAY_SETTINGS_SCHEMA_VERSION));
                let legacy_run_context = !current_schema && has("encounter");
                let shows_outgoing_metric = has("dps") || has("damage");
                let shows_player_metric = shows_outgoing_metric || has("incoming");
                base["vr_overlay"]["show_rolling_dps"] = Value::Bool(has("dps"));
                base["vr_overlay"]["show_total_damage"] = Value::Bool(has("damage"));
                base["vr_overlay"]["show_incoming"] = Value::Bool(has("incoming"));
                base["vr_overlay"]["show_players"] = Value::Bool(shows_player_metric);
                base["vr_overlay"]["show_recent_hits"] =
                    Value::Bool(has("recent_hits") || has("hits"));
                base["vr_overlay"]["show_encounter"] = Value::Bool(has("encounter"));
                base["vr_overlay"]["show_phase"] = Value::Bool(has("phase") || legacy_run_context);
                base["vr_overlay"]["show_boss_number"] =
                    Value::Bool(has("boss") || legacy_run_context);
                base["vr_overlay"]["show_focus"] = Value::Bool(has("focus"));
                base["vr_overlay"]["show_loadout"] = Value::Bool(has("loadout"));
                base["vr_overlay"]["show_attacks"] = Value::Bool(shows_outgoing_metric);
            }
        }
    }
}

fn deep_merge(base: &mut Value, patch: Value) {
    match (base, patch) {
        (Value::Object(base), Value::Object(patch)) => {
            for (key, value) in patch {
                if let Some(existing) = base.get_mut(&key) {
                    deep_merge(existing, value);
                } else {
                    base.insert(key, value);
                }
            }
        }
        (base, patch) => *base = patch,
    }
}

fn persist_uploaded_log(
    directory: &std::path::Path,
    bytes: &[u8],
    fingerprint: &str,
) -> anyhow::Result<PathBuf> {
    // Party members sometimes export a log while VRChat is still appending to it, then upload
    // the completed version later. Keep a stable source path for exact prefix/superset versions
    // so storage can replace that source instead of counting both snapshots as two players.
    if directory.exists() && !bytes.is_empty() {
        for entry in std::fs::read_dir(directory)? {
            let entry = entry?;
            let candidate = entry.path();
            let is_import = candidate
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("sha256-") && name.ends_with(".txt"));
            if !is_import {
                continue;
            }
            let existing = std::fs::read(&candidate)?;
            if existing == bytes || existing.len() > bytes.len() && existing.starts_with(bytes) {
                return Ok(candidate);
            }
            if bytes.len() > existing.len() && bytes.starts_with(&existing) {
                std::fs::write(&candidate, bytes)?;
                return Ok(candidate);
            }
        }
    }

    let path = directory.join(format!("sha256-{fingerprint}.txt"));
    if path.exists() {
        let existing = std::fs::read(&path)?;
        if content_fingerprint(&existing) == *fingerprint {
            return Ok(path);
        }
        tracing::warn!(path = %path.display(), "repairing incomplete content-addressed import");
    }
    std::fs::write(&path, bytes)?;
    Ok(path)
}

fn find_monitored_duplicate(
    directory: &std::path::Path,
    expected_size: u64,
    expected_fingerprint: &str,
) -> anyhow::Result<Option<PathBuf>> {
    if !directory.exists() {
        return Ok(None);
    }
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let is_vrchat_log = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("output_log_") && name.ends_with(".txt"));
        if !is_vrchat_log || entry.metadata()?.len() != expected_size {
            continue;
        }
        if file_fingerprint(&path)? == expected_fingerprint {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn file_fingerprint(path: &std::path::Path) -> anyhow::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = hasher.finalize();
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn content_fingerprint(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn expand_profile(value: &str) -> String {
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        return value.replace("%USERPROFILE%", &profile.to_string_lossy());
    }
    value.to_owned()
}

fn error_response(status: StatusCode, message: String) -> Response {
    (status, Json(json!({ "ok": false, "error": message }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_directory(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "minmaxxer-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn uploads_are_content_addressed_and_reuse_the_same_path() {
        let directory = temporary_directory("upload-fingerprint");
        std::fs::create_dir_all(&directory).unwrap();
        let bytes = b"same Ecliptica log bytes";
        let fingerprint = content_fingerprint(bytes);
        let first = persist_uploaded_log(&directory, bytes, &fingerprint).unwrap();
        let second = persist_uploaded_log(&directory, bytes, &fingerprint).unwrap();
        assert_eq!(first, second);
        assert_eq!(std::fs::read(&first).unwrap(), bytes);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn progressive_uploads_keep_one_source_and_the_longest_content() {
        let directory = temporary_directory("progressive-upload");
        std::fs::create_dir_all(&directory).unwrap();
        let partial = b"Ecliptica log line one\npartial";
        let complete = b"Ecliptica log line one\npartial log line two\n";
        let first =
            persist_uploaded_log(&directory, partial, &content_fingerprint(partial)).unwrap();
        let upgraded =
            persist_uploaded_log(&directory, complete, &content_fingerprint(complete)).unwrap();
        let older_again =
            persist_uploaded_log(&directory, partial, &content_fingerprint(partial)).unwrap();
        assert_eq!(first, upgraded);
        assert_eq!(first, older_again);
        assert_eq!(std::fs::read(&first).unwrap(), complete);
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 1);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn identical_monitored_log_wins_over_an_uploaded_copy() {
        let directory = temporary_directory("monitored-fingerprint");
        std::fs::create_dir_all(&directory).unwrap();
        let bytes = b"monitored Ecliptica log bytes";
        let monitored = directory.join("output_log_2026-07-21_20-00-00.txt");
        std::fs::write(&monitored, bytes).unwrap();
        let found =
            find_monitored_duplicate(&directory, bytes.len() as u64, &content_fingerprint(bytes))
                .unwrap();
        assert_eq!(found.as_deref(), Some(monitored.as_path()));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn host_guard_accepts_only_the_loopback_names_used_by_the_app() {
        assert!(is_allowed_host("127.0.0.1:49321"));
        assert!(is_allowed_host("localhost:49321"));
        assert!(!is_allowed_host("minmaxxer.attacker.example:49321"));
        assert!(!is_allowed_host("127.0.0.1.attacker.example"));
    }

    #[test]
    fn embedded_overlay_ships_a_dark_pre_script_local_service_fallback() {
        let script_start = OVERLAY_HTML
            .find("<script")
            .expect("the embedded overlay should load its application script");
        let pre_script_markup = &OVERLAY_HTML[..script_start];
        let normalized_markup = pre_script_markup.to_ascii_lowercase();

        assert!(pre_script_markup.contains("CONNECTING TO LOCAL SERVICE"));
        assert!(normalized_markup.contains("color-scheme:dark"));
        assert!(normalized_markup.contains("background:"));
        assert!(!pre_script_markup.contains("DEMO DATA"));
        assert!(!pre_script_markup.contains("Astral Sovereign"));
        assert!(!pre_script_markup.contains("48,240"));
    }

    #[test]
    fn main_ui_tells_obs_users_to_replace_the_entire_default_url() {
        let normalized_html = INDEX_HTML.to_ascii_lowercase();

        assert!(INDEX_HTML.contains("Ctrl+A"));
        assert!(normalized_html.contains("obs"));
        assert!(normalized_html.contains("replace the entire"));
        assert!(normalized_html.contains("url"));
    }

    #[test]
    fn obs_runtime_uses_truthful_idle_and_disconnect_states_without_demo_combat() {
        let boot_start = APP_JS
            .find("async function bootOverlay()")
            .expect("the OBS overlay bootstrap should exist");
        let boot_end = APP_JS[boot_start..]
            .find("async function bootApp()")
            .map(|offset| boot_start + offset)
            .expect("the main app bootstrap should follow the overlay bootstrap");
        let overlay_boot = &APP_JS[boot_start..boot_end];

        assert!(APP_JS.contains("NO LIVE ECLIPTICA INSTANCE"));
        assert!(APP_JS.contains("LOCAL SERVICE DISCONNECTED"));
        assert!(APP_JS.contains("RECONNECTING"));
        assert!(overlay_boot.contains("makeOverlayWaitingLive"));
        assert!(!overlay_boot.contains("makeMockLive"));
        assert!(!overlay_boot.contains("startDemoClock"));
    }

    #[test]
    fn main_live_runtime_uses_real_or_idle_data_and_clears_on_stream_loss() {
        let load_start = APP_JS
            .find("async function loadInitialData()")
            .expect("the main data bootstrap should exist");
        let load_end = APP_JS[load_start..]
            .find("function updateConnectionUI")
            .map(|offset| load_start + offset)
            .expect("connection rendering should follow the bootstrap");
        let bootstrap = &APP_JS[load_start..load_end];
        let stream_start = APP_JS
            .find("function connectStream")
            .expect("the stream connector should exist");
        let stream_end = APP_JS[stream_start..]
            .find("async function refreshRuns")
            .map(|offset| stream_start + offset)
            .expect("run refresh should follow the stream connector");
        let stream = &APP_JS[stream_start..stream_end];

        assert!(bootstrap.contains("makeOverlayWaitingLive(state.apiOnline)"));
        assert!(bootstrap.contains("state.usingMock = false"));
        assert!(!bootstrap.contains("makeMockLive"));
        assert!(!bootstrap.contains("makeMockRuns"));
        assert!(stream.contains("clearLiveOnError"));
        assert!(stream.contains("state.live = makeOverlayWaitingLive(false)"));
        assert!(stream.contains("state.lastLiveAt = 0"));
    }

    #[test]
    fn studio_hydrates_backend_profile_and_disables_unavailable_loadout_control() {
        let hydrate_start = APP_JS
            .find("function studioOptionsFromSettings")
            .expect("backend Studio hydration should exist");
        let hydrate_end = APP_JS[hydrate_start..]
            .find("async function setOutputEnabled")
            .map(|offset| hydrate_start + offset)
            .expect("output controls should follow Studio hydration");
        let hydration = &APP_JS[hydrate_start..hydrate_end];

        assert!(hydration.contains("state.settings?.overlay_profiles"));
        assert!(hydration.contains("studioOptionsFromSettings() || local"));
        assert!(hydration.contains("!input.disabled"));
        assert!(APP_JS.contains("function queueOverlayProfileSave(options)"));
        assert!(!APP_JS.contains("if (!outputsEnabled()) return"));
        assert!(hydration.contains("profile.accent === \"#8ff0cf\" ? \"mint\""));
        assert!(INDEX_HTML.contains("Loadout unavailable (not logged)"));
        assert!(INDEX_HTML.contains("value=\"loadout\" disabled"));
    }

    #[test]
    fn settings_ui_wires_the_native_boss_target_alert_toggle() {
        assert!(INDEX_HTML.contains("id=\"bossTargetAlertToggle\""));
        assert!(INDEX_HTML.contains("Boss-target sound alert"));
        assert!(APP_JS.contains("state.settings.boss_target_alert_enabled ?? true"));
        assert!(APP_JS.contains(
            "boss_target_alert_enabled: $(\"#bossTargetAlertToggle\").getAttribute(\"aria-checked\") === \"true\""
        ));
        assert!(APP_JS.contains(
            "\"#autoImportToggle\", \"#bossTargetAlertToggle\", \"#launchMinimizedToggle\""
        ));
    }

    #[test]
    fn legacy_obs_only_studio_profile_migrates_before_backend_becomes_canonical() {
        let hydrate_start = APP_JS
            .find("function applySavedStudioOptions")
            .expect("Studio saved-option hydration should exist");
        let hydrate_end = APP_JS[hydrate_start..]
            .find("async function setOutputEnabled")
            .map(|offset| hydrate_start + offset)
            .expect("output controls should follow saved-option hydration");
        let hydration = &APP_JS[hydrate_start..hydrate_end];
        let save_start = APP_JS
            .find("function queueOverlayProfileSave")
            .expect("Studio backend save should exist");
        let save_end = APP_JS[save_start..]
            .find("function studioOptionsFromSettings")
            .map(|offset| save_start + offset)
            .expect("backend hydration should follow the save helpers");
        let save_helpers = &APP_JS[save_start..save_end];

        assert!(hydration.contains("localVersion < OVERLAY_SETTINGS_VERSION"));
        assert!(hydration.contains("Boolean(local?.backendPending)"));
        assert!(hydration
            .contains("migrateLocal || retryLocal ? local : studioOptionsFromSettings() || local"));
        assert!(hydration.contains("restoredShow.add(\"phase\")"));
        assert!(hydration.contains("restoredShow.add(\"boss\")"));
        assert!(hydration.contains("queueOverlayProfileSave(migrated)"));
        assert!(save_helpers.contains("backendPending: state.studioBackendPending"));
        assert!(save_helpers.contains("state.studioBackendPending = false"));
        assert!(save_helpers.contains("state.profileSavePendingOptions = { ...options }"));
        assert!(save_helpers
            .contains("if (state.profileSaveInFlight || !state.profileSavePendingOptions) return"));
        assert!(save_helpers.contains("if (!state.profileSavePendingOptions)"));
        assert!(save_helpers
            .contains("state.profileSaveTimer = setTimeout(flushOverlayProfileSave, 0)"));
    }

    #[test]
    fn browser_overlay_urls_are_versioned_and_legacy_encounter_urls_gain_run_context() {
        let options_start = APP_JS
            .find("function overlayOptionsFromSearch")
            .expect("the overlay URL parser should exist");
        let options_end = APP_JS[options_start..]
            .find("function renderCombatOverlay")
            .map(|offset| options_start + offset)
            .expect("the overlay renderer should follow its URL parser");
        let options_parser = &APP_JS[options_start..options_end];
        let url_start = APP_JS
            .find("function overlayUrl")
            .expect("the Studio URL builder should exist");
        let url_end = APP_JS[url_start..]
            .find("function renderVrStatus")
            .map(|offset| url_start + offset)
            .expect("VR status rendering should follow the URL builder");
        let url_builder = &APP_JS[url_start..url_end];

        assert!(APP_JS.contains("const OVERLAY_SETTINGS_VERSION = 4"));
        assert!(url_builder.contains("params.set(\"ui\", OVERLAY_SETTINGS_VERSION)"));
        assert!(options_parser.contains("params.get(\"ui\") !== String(OVERLAY_SETTINGS_VERSION)"));
        assert!(options_parser.contains("params.has(\"show\")"));
        assert!(options_parser.contains("parsedShow.includes(\"encounter\")"));
        assert!(options_parser.contains("parsedShow.splice(telemetryIndex, 1)"));
        assert!(options_parser.contains("parsedShow.push(\"phase\")"));
        assert!(options_parser.contains("parsedShow.push(\"boss\")"));
    }

    #[test]
    fn studio_starts_at_actual_pixels_and_hides_unavailable_placeholders() {
        assert!(INDEX_HTML.contains("id=\"previewOneToOne\" class=\"active\""));
        assert!(INDEX_HTML.contains("class=\"overlay-stage one-to-one\""));
        assert!(INDEX_HTML.contains("value=\"telemetry\"><span>"));
        assert!(!INDEX_HTML.contains("value=\"telemetry\" checked"));
        assert!(APP_JS.contains("restoredShow.delete(\"telemetry\")"));
        assert!(APP_JS.contains("preview.style.transform = oneToOne ? \"scale(1)\""));
        assert!(STYLE_CSS.contains(".overlay-stage.one-to-one"));
        assert!(STYLE_CSS.contains("max-width:2400px"));
        assert!(STYLE_CSS.contains("height:min(722px"));
        assert!(STYLE_CSS.contains("z-index:2;grid-area:1/1"));
    }

    #[test]
    fn browser_overlay_allows_context_only_and_explicitly_empty_visibility() {
        let options_start = APP_JS
            .find("function overlayOptionsFromSearch")
            .expect("the overlay URL parser should exist");
        let options_end = APP_JS[options_start..]
            .find("function metricValue")
            .map(|offset| options_start + offset)
            .expect("the metric helper should follow the URL parser");
        let options_parser = &APP_JS[options_start..options_end];
        let render_start = APP_JS
            .find("function renderCombatOverlay")
            .expect("the overlay renderer should exist");
        let render_end = APP_JS[render_start..]
            .find("function hexToRgb")
            .map(|offset| render_start + offset)
            .expect("the color helper should follow the overlay renderer");
        let renderer = &APP_JS[render_start..render_end];

        assert!(options_parser.contains("const hasShow = params.has(\"show\")"));
        assert!(options_parser.contains("const parsedShow = hasShow ? showRaw.split"));
        assert!(!renderer.contains("metrics.push(\"dps\")"));
        assert!(renderer.contains("LAST 5 LOCAL DAMAGE EVENTS"));
        assert!(renderer.contains("SURVIVAL FEED"));
        assert!(renderer.contains("dpsGraphMarkup(live, scope)"));
        assert!(renderer.contains("HP REMAINING"));
    }

    #[test]
    fn overlay_patch_schema_distinguishes_legacy_inheritance_from_explicit_controls() {
        fn profile(base: &Value) -> &Value {
            base["overlay_profiles"]
                .as_array()
                .unwrap()
                .iter()
                .find(|profile| profile["id"] == "broadcast")
                .unwrap()
        }

        let apply = |schema_version: Option<u64>, show: Vec<&str>| {
            let mut base = serde_json::to_value(AppConfig::default()).unwrap();
            let mut overlay = json!({ "show": show });
            if let Some(schema_version) = schema_version {
                overlay["schema_version"] = json!(schema_version);
            }
            let patch = json!({ "overlay": overlay });
            apply_compatibility_aliases(&mut base, &patch);
            base
        };

        let legacy = apply(None, vec!["encounter"]);
        assert_eq!(profile(&legacy)["show_phase"], true);
        assert_eq!(profile(&legacy)["show_boss_number"], true);
        assert_eq!(profile(&legacy)["show_loadout"], false);
        assert_eq!(legacy["vr_overlay"]["show_phase"], true);
        assert_eq!(legacy["vr_overlay"]["show_boss_number"], true);
        assert_eq!(legacy["vr_overlay"]["show_loadout"], false);

        let explicit = apply(Some(4), vec!["encounter"]);
        assert_eq!(profile(&explicit)["show_phase"], false);
        assert_eq!(profile(&explicit)["show_boss_number"], false);
        assert_eq!(profile(&explicit)["show_telemetry"], false);
        assert_eq!(profile(&explicit)["show_loadout"], false);
        assert_eq!(explicit["vr_overlay"]["show_phase"], false);
        assert_eq!(explicit["vr_overlay"]["show_boss_number"], false);
        assert_eq!(explicit["vr_overlay"]["show_loadout"], false);
        assert_eq!(explicit["overlay_schema_version"], 4);

        let loadout = apply(Some(4), vec!["encounter", "phase", "boss", "loadout"]);
        assert_eq!(profile(&loadout)["show_phase"], true);
        assert_eq!(profile(&loadout)["show_boss_number"], true);
        assert_eq!(profile(&loadout)["show_loadout"], true);
        assert_eq!(loadout["vr_overlay"]["show_phase"], true);
        assert_eq!(loadout["vr_overlay"]["show_boss_number"], true);
        assert_eq!(loadout["vr_overlay"]["show_loadout"], true);

        let context_only = apply(Some(4), vec!["phase", "boss", "hits", "focus"]);
        assert_eq!(profile(&context_only)["show_dps"], false);
        assert_eq!(profile(&context_only)["show_damage"], false);
        assert_eq!(context_only["vr_overlay"]["show_players"], false);
        assert_eq!(context_only["vr_overlay"]["show_attacks"], false);
        assert_eq!(context_only["vr_overlay"]["show_rolling_dps"], false);
        assert_eq!(context_only["vr_overlay"]["show_total_damage"], false);

        let incoming_only = apply(Some(4), vec!["incoming"]);
        assert_eq!(incoming_only["vr_overlay"]["show_incoming"], true);
        assert_eq!(incoming_only["vr_overlay"]["show_players"], true);
        assert_eq!(incoming_only["vr_overlay"]["show_attacks"], false);
        assert_eq!(incoming_only["vr_overlay"]["show_rolling_dps"], false);
        assert_eq!(incoming_only["vr_overlay"]["show_total_damage"], false);

        let old_telemetry = apply(Some(3), vec!["telemetry"]);
        assert_eq!(profile(&old_telemetry)["show_telemetry"], false);

        let opted_in_telemetry = apply(Some(4), vec!["telemetry"]);
        assert_eq!(profile(&opted_in_telemetry)["show_telemetry"], true);
    }

    #[test]
    fn studio_profile_and_background_reach_native_outputs() {
        let mut base = serde_json::to_value(AppConfig::default()).unwrap();
        let patch = json!({
            "overlay": {
                "profile": "minimal",
                "layout": "landscape",
                "theme": "glass",
                "accent": "mint",
                "rows": 3,
                "hit_rows": 2,
                "scale": 115,
                "bg": 42,
                "show": ["dps", "hits", "focus"]
            }
        });
        apply_compatibility_aliases(&mut base, &patch);

        assert_eq!(base["desktop_overlay"]["profile"], "minimal");
        assert_eq!(base["desktop_overlay"]["opacity"], 0.42);
        assert!((base["vr_overlay"]["opacity"].as_f64().unwrap() - 1.0).abs() < 0.0001);
        assert_eq!(base["vr_overlay"]["rows"], 3);
        assert_eq!(base["vr_overlay"]["recent_hit_rows"], 2);
        let profile = base["overlay_profiles"]
            .as_array()
            .unwrap()
            .iter()
            .find(|profile| profile["id"] == "minimal")
            .unwrap();
        assert_eq!(profile["layout"], "landscape");
        assert_eq!(profile["theme"], "glass");
        assert_eq!(profile["accent"], "mint");
        assert_eq!(profile["scale"], 1.15);
    }
}
