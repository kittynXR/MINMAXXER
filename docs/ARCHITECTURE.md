# Architecture

```text
VRChat output_log_*.txt
        |
  250 ms incremental tail (shared read, rotation/truncation recovery)
        |
  Ecliptica stateful parser ---- unknown relevant lines
        |                              |
  normalized GameEvent           diagnostics table
        |
  SQLite WAL + live CombatEngine
        |
  loopback Axum API / SSE on 127.0.0.1:49321
        |
        +-- Tauri/WebView2 analysis app
        +-- OBS browser source
        +-- click-through VRChat desktop overlay
        +-- native OpenVR RGBA overlay
```

## Resource choices

- The collector blocks on a channel with a 250 ms timeout and performs no work when the log has not grown.
- Parsing is line-oriented and incremental; the active partial line and a 64 KiB read buffer are the only file buffers.
- SQLite uses WAL mode and batched inserts keyed by source path and byte offset.
- Live clients share a Tokio watch channel, so only the newest snapshot is retained.
- OBS and desktop overlays reuse the same loopback HTML/CSS/JavaScript assets.
- The VR overlay avoids a second browser process. It rasterizes a dense HUD with `fontdue`, reuses a 1024 × 512 RGBA allocation, redraws only on changes, and caps OpenVR uploads at 4 Hz. Width, opacity, curvature, and transform writes are cached separately so a texture redraw does not reconfigure the compositor.
- SteamVR is not initialized until the VR output is enabled.

## Process boundaries

`minmaxxer-core` owns parsing, normalized models, live aggregation, focus inference, and run analysis. It has no Tauri, database, web-server, or OpenVR dependency and can be regression-tested with log fixtures.

Historical run aggregation treats boss windows as the primary analysis scope whenever boss-start telemetry is present. Pre-boss activity is accumulated separately. Boss ends prefer matching named death/summary records, with later boss, intermission, lobby, world-exit, and stage boundaries retained as structural fallbacks. Live target inference is an experimental, exact-name network-ownership proxy with optional incoming-hit corroboration, confidence/evidence/age metadata, and encounter-boundary lifetime; it is not an authoritative hate source.

The `minmaxxer` app owns Windows integration, persistence, the loopback API, file imports, WebView windows, and output lifecycle. The HTTP listener is bound before windows are created so startup fails clearly instead of silently selecting a different OBS URL.

## Local API

| Endpoint | Purpose |
|---|---|
| `GET /api/health` | Version, uptime, and storage counts. |
| `GET /api/live` | Latest normalized live snapshot. |
| `GET /api/stream` | Server-sent snapshot changes plus keep-alives. |
| `GET /api/runs` | Post-run summaries, newest first. |
| `GET /api/runs/{id}` | One run and its normalized events. |
| `GET /api/events` | Raw normalized event explorer. |
| `GET /api/vr-status` | Native OpenVR worker, visibility, placement, and frame status. |
| `GET/PUT /api/settings` | Persisted overlay and collector configuration. |
| `POST /api/import` | Multipart VRChat log import. |
| `POST /api/rescan` | Rescan configured recent logs. |

Static responses use a restrictive content-security policy and `no-store`. The server rejects non-loopback `Host` values to prevent DNS rebinding. Mutating endpoints require the installation's OS-random `X-MINMAXXER-Token`, which the same-origin app reads from settings; read-only OBS/SSE endpoints remain token-free. Uploaded logs are SHA-256-addressed inside the application import directory and identical monitored/imported files are reused instead of double-counted. The service is intentionally loopback-only.

The health endpoint reports API version `2`; this version makes boss windows the primary run-summary scope and exposes the observed/pre-boss fields needed to audit that scope.
