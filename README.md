# MINMAXXER

MINMAXXER is a lightweight Windows combat HUD and run analyzer for the VRChat world **Ecliptica**. It tails VRChat's text log, stores normalized events in a local SQLite database, and serves one live data stream to the desktop app, OBS browser sources, the desktop game overlay, and a native SteamVR overlay.

Everything stays on the PC. The web server listens only on `127.0.0.1`.

All application and HUD surfaces are dark-only. The Windows title bar, WebView prepaint, dashboard, OBS controls, desktop HUD, and native VR panel are forced dark or transparent; there is no light theme.

## What it reports

- Exact outgoing hit values for the player whose log is being watched, split into strike and non-strike damage.
- A configurable, newest-first phase-hit feed for classes whose damage numbers are obscured by movement or VFX. The last hits remain until the current phase boundary and dim after inactivity instead of disappearing.
- Boss-window, encounter, rolling 5/15/30-second, stage, and run damage statistics. When boss telemetry exists, the primary historical DPS and totals cover boss fights only; pre-boss time and damage are reported separately.
- Incoming damage totals, largest hit, and breakdown by the raw attack/source label present in the log.
- Healing and HP-gain surfaces are explicitly marked unavailable and disabled until a real vanilla Ecliptica log exposes attributable values; reserved fields remain in the data model for forward compatibility.
- Dense stream/VR HUDs with prominent MINMAXXER branding, a current-segment rolling DPS graph plus full-phase average, the last five local damage events, and a separate incoming-damage feed.
- Stage, class, boss/phase, session, roster, encounter timing, and raw event history.
- Live run context with the PRIME/PENUMBRA/ANTUMBRA/UMBRA/ECLIPSE band and the current 1-based boss ordinal. Mid-run ordinal recovery is visibly marked as inferred.
- Multi-player post-run reports after logs from the other players are imported. Matching Ecliptica session IDs are merged automatically.
- An always-visible experimental `BOSS TARGET?` signal based on exact boss-entity VRChat network ownership, with confidence, evidence, and signal age. A matching incoming hit can corroborate the proxy, and the last candidate remains visible until the encounter boundary. Network ownership is **not** an authoritative hate table.
- A target-only 720 × 180 browser/desktop HUD and matching native VR presentation for players who only need the current boss target. Same-encounter player handoffs use a low-motion, reduced-motion-aware visual transition.
- Native audio cues when a fresh `BOSS TARGET?` signal switches to the local player and when a later live ownership transfer moves that same boss to another known player. The urgent warning and softer all-clear are enabled together by default. A separate, gentle synthesized cue for other player-to-player handoffs is optional and off by default. All cues work while minimized and in headless mode and can be routed to any active Windows audio output in Settings.

Ecliptica directly logs a numeric run-progress value, but not the named difficulty band. MINMAXXER maps it with the documented, unofficial [EACT five-band convention](https://eact-doc.rtail.dev/?lang=en#phases): `0.0–<0.2` PRIME, `0.2–<0.4` PENUMBRA, `0.4–<0.6` ANTUMBRA, `0.6–<0.8` UMBRA, and `0.8–1.0` ECLIPSE. The half-open endpoint handling is MINMAXXER's convention, not an official world API contract. At progress `1.0`, a matching final Bringer-stage marker is presented as **EYE OF THE ECLIPSE**, corroborated by the community [boss reference](https://wikiwiki.jp/ecliptica/%E3%83%9C%E3%82%B9), while retaining the ECLIPSE numeric band. Boss ordinals counted from a run observed at progress zero are exact, as is boss 13 when the final Bringer marker identifies it; other mid-run recovery uses audited progress anchors and labels the estimate `INFERRED`/`~`.

Ecliptica's audited vanilla logs do **not** expose other players' damage to one client, player/enemy HP, healing, shields, critical hits, blocks, misses, authoritative kills, applied/removed buff and debuff state, gem balances/spending, shop item identities and stack counts, or the current song. Healing controls and values are therefore grayed out and cannot be enabled; stale saved preferences remain loadable but are capability-gated by every renderer. Session load/save markers are checkpoints, not item-purchase records, so MINMAXXER does not construct a loadout from them. MINMAXXER never invents unavailable values. Unsupported placeholders are hidden on every default HUD so they consume no live space; the web overlay can show an optional reminder row. See [Log telemetry](docs/LOG_TELEMETRY.md) for the full capability matrix.

## Run it

1. Start `minmaxxer.exe`. It automatically finds `%USERPROFILE%\AppData\LocalLow\VRChat\VRChat` and imports recent logs.
2. Start VRChat normally. The live display updates as Ecliptica writes complete log lines.
3. The first-run default is **Target only**, a compact boss-target HUD. Open **Overlay studio** to switch to Broadcast, Minimal, or VR glance when you want DPS and other combat detail, or to tune colors, scale, visible sections, and recent-hit count. Existing installations keep their saved profile choice. The preview starts at **1:1 actual pixels** for readable large-screen inspection; choose **Fit** when a scaled overview is more useful.

The portable release executable statically links the Visual C++ runtime. Its only desktop UI prerequisite is the Microsoft Edge WebView2 runtime included with current Windows 10/11 installations. SteamVR is only initialized when its overlay is enabled. Published builds are currently unsigned, so Windows SmartScreen may ask for confirmation on first launch.

The embedded boss-target warning and all-clear are Kenney's CC0 `error_006` and `confirmation_001` interface sounds, converted to small PCM WAV files for native Windows playback. See [Third-party notices](THIRD_PARTY_NOTICES.md) for their source and license.

### OBS

Add a **Browser** source and use:

```text
http://127.0.0.1:49321/overlay
```

The bare URL opens the default **Target only** profile; use **720 × 180** for it. Overlay Studio can generate a full Broadcast or other profile URL, using **1280 × 720** for landscape or **720 × 1280** for portrait. In OBS's URL field, press **Ctrl+A before pasting** so the MINMAXXER address replaces the entire default `https://obsproject.com/browser-source` value instead of being appended to it. Overlay studio generates the complete URL and shows the matching Browser Source size. For example, portrait uses:

```text
http://127.0.0.1:49321/overlay?profile=broadcast&layout=portrait
```

Keep MINMAXXER running while OBS uses the source; the server and collector continue running when the main window is minimized to the tray. With no active VRChat log, the source shows a branded `MINMAXXER READY · NO LIVE ECLIPTICA INSTANCE` card. If the service connection is lost after the page loads, it switches to a reconnecting card instead of showing demo or stale combat values.

### Desktop overlay

Enable **Desktop overlay** in Overlay studio. The click-through WebView follows the VRChat window and hides when VRChat is minimized or not foreground (configurable). By default it also hides whenever OpenVR detects a headset; enable **Show desktop HUD with a detected headset** when an attached but idle headset would otherwise suppress desktop-mode play.

### SteamVR overlay

Enable **VR overlay** while SteamVR is running. Fresh installs start with Target only: one centered, large-name boss-target panel. Choosing Broadcast or another detail profile restores the DPS graph and combat feeds. The 1024 × 512 HUD is rendered directly into a reusable RGBA buffer and uploaded only when content changes, capped at 4 Hz. Each asynchronous upload keeps immutable backing memory until SteamVR reports completion; visibility events and a heartbeat reconcile the real compositor state without redundant show calls. Unavailable telemetry consumes no native panel space. Its HMD-relative X/Y/Z position, width, curvature, and opacity are persisted in settings. Compositor presentation and transform values are cached so ordinary content redraws do not repeatedly rewrite them.

Turn on **Grab & move overlay** to unlock placement. Squeeze the right grip to attach the HUD, move it naturally, and release to place it; the overlay remains available for another grab until the toggle is turned off. Turning the toggle off immediately finalizes the current placement and locks the HUD against accidental movement. Haptics acknowledge grab and placement. A controller-placed transform survives redraws and SteamVR reconnects for the current app session; use the saved X/Y/Z controls for placement that must survive an app restart. Hardware presentation still needs to be checked on each SteamVR/runtime combination.

## Merge a party run

Each player runs MINMAXXER during the run, or sends their `output_log_*.txt` afterward. In **Run history**, choose **Import log** for each file. The analyzer associates player-local damage with that log's local player and merges records with the same valid Ecliptica session ID. Instance and visit timing are retained for diagnosing blank or stale session IDs.

Do not interpret roster names or network ownership as damage attribution. Ecliptica does not place the remote attacker on damage lines.

Historical run analysis prioritizes optimization time: if a run contains boss-start telemetry, its primary duration, DPS, damage, attacks, and player totals include only the boss windows. Pre-boss combat remains available as a separate subtotal instead of being blended into boss performance. A boss window ends at the first reliable boundary, preferring a matching named boss-death or personal-summary record, then a following boss start, intermission, lobby, world exit, or stage transition. Damage recorded after that collector's boundary remains available in raw observed telemetry but cannot change the completed boss's timer, graph, totals, or DPS. The analyzer falls back to the observed combat span only when no boss windows are present.

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

The normal executable is written to `target\release\minmaxxer.exe`. The portable release script uses a clean isolated target, statically links the MSVC runtime, strips local build paths, and writes `dist\MINMAXXER-v0.4.6-windows-x64.exe`. Run the local server without the desktop WebView with `minmaxxer.exe --headless`.

The code is split into a dependency-light parser/analytics crate and a Windows app crate. More detail is in [Architecture](docs/ARCHITECTURE.md).
