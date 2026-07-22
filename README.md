# MINMAXXER

MINMAXXER is a lightweight Windows combat HUD and run analyzer for the VRChat world **Ecliptica**. It tails VRChat's text log, stores normalized events in a local SQLite database, and serves one live data stream to the desktop app, OBS browser sources, the desktop game overlay, and a native SteamVR overlay.

Everything stays on the PC. The web server listens only on `127.0.0.1`.

All application and HUD surfaces are dark-only. The Windows title bar, WebView prepaint, dashboard, OBS controls, desktop HUD, and native VR panel are forced dark or transparent; there is no light theme.

## What it reports

- Exact outgoing hit values for the player whose log is being watched, split into strike and non-strike damage.
- A configurable, newest-first recent-hit feed for classes whose damage numbers are obscured by movement or VFX.
- Encounter, rolling 5/15/30-second, stage, boss-phase, and full-run damage statistics.
- Incoming damage totals, largest hit, and breakdown by the raw attack/source label present in the log.
- Stage, class, boss/phase, session, roster, encounter timing, and raw event history.
- Multi-player post-run reports after logs from the other players are imported. Matching Ecliptica session IDs are merged automatically.
- An experimental `FOCUS?` signal when a boss-matching entity's VRChat network ownership moves to a player. It is deliberately labeled as inferred and expires quickly; network ownership is **not** an authoritative hate table.

Ecliptica's current logs do **not** expose other players' damage to one client, player/enemy HP, healing, shields, critical hits, blocks, misses, authoritative kills, or applied/removed buff and debuff state. MINMAXXER never invents those values. See [Log telemetry](docs/LOG_TELEMETRY.md) for the full capability matrix.

## Run it

1. Start `minmaxxer.exe`. It automatically finds `%USERPROFILE%\AppData\LocalLow\VRChat\VRChat` and imports recent logs.
2. Start VRChat normally. The live display updates as Ecliptica writes complete log lines.
3. Open **Overlay studio** to choose the compact HUD layout, colors, scale, visible rows, recent-hit count, and optional `FOCUS?` item.

The portable release executable statically links the Visual C++ runtime. Its only desktop UI prerequisite is the Microsoft Edge WebView2 runtime included with current Windows 10/11 installations. SteamVR is only initialized when its overlay is enabled. Published builds are currently unsigned, so Windows SmartScreen may ask for confirmation on first launch.

### OBS

Add a **Browser** source and use:

```text
http://127.0.0.1:49321/overlay
```

Use a transparent source around 720 × 480, then customize the URL from Overlay studio. A hit-feed-only source can use:

```text
http://127.0.0.1:49321/overlay?layout=hits&hit_rows=6
```

The server and collector keep running when the main window is minimized to the tray.

### Desktop overlay

Enable **Desktop overlay** in Overlay studio. The click-through WebView follows the VRChat window and hides when VRChat is minimized or not foreground (configurable). By default it also hides whenever OpenVR detects a headset; enable **Show desktop HUD with a detected headset** when an attached but idle headset would otherwise suppress desktop-mode play.

### SteamVR overlay

Enable **VR overlay** while SteamVR is running. The HUD is rendered directly into a reusable RGBA buffer and uploaded only when content changes, capped at 4 Hz. Its HMD-relative X/Y/Z position, width, curvature, and opacity are persisted in settings.

Optional controller placement uses a deliberate gesture so an ordinary one-hand grab in VRChat cannot move the HUD: hold both controller grips for about 0.9 seconds, move the attached panel with the right controller, then release the right grip to freeze it in the current HMD-relative position. Haptics acknowledge arming and placement. A controller-placed transform survives redraws and SteamVR reconnects for the current app session; use the saved X/Y/Z controls for placement that must survive an app restart. Hardware presentation still needs to be checked on each SteamVR/runtime combination.

## Merge a party run

Each player runs MINMAXXER during the run, or sends their `output_log_*.txt` afterward. In **Run history**, choose **Import log** for each file. The analyzer associates player-local damage with that log's local player and merges records with the same valid Ecliptica session ID. Instance and visit timing are retained for diagnosing blank or stale session IDs.

Do not interpret roster names or network ownership as damage attribution. Ecliptica does not place the remote attacker on damage lines.

## Data and settings

Local files are stored under `%LOCALAPPDATA%\MINMAXXER`:

- `config.json` — log location, overlay profiles, and output settings.
- `combat.sqlite` — normalized event history (SQLite WAL mode).
- `imports\` — copies of logs imported through the UI.

The parser retains source byte offsets and never deduplicates equal-looking hit lines: several legitimate hits can have the same second, type, and amount. Unknown Ecliptica-shaped lines are quarantined for parser diagnostics.

## Build from source

Requirements: Rust with the MSVC target, Visual Studio C++ build tools, and WebView2.

```powershell
cargo test --workspace
cargo build --release -p minmaxxer
./scripts/build-portable.ps1
```

The normal executable is written to `target\release\minmaxxer.exe`. The portable release script uses a clean isolated target, statically links the MSVC runtime, strips local build paths, and writes `dist\MINMAXXER-v0.1.0-windows-x64.exe`. Run the local server without the desktop WebView with `minmaxxer.exe --headless`.

The code is split into a dependency-light parser/analytics crate and a Windows app crate. More detail is in [Architecture](docs/ARCHITECTURE.md).
