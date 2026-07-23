//! Low-overhead SteamVR overlay for the live Ecliptica combat snapshot.
//!
//! OpenVR is deliberately initialized only while [`VrOverlaySettings::enabled`] is true. The
//! worker keeps texture uploads change-driven, limits them to four per second, and only polls the
//! tiny legacy controller-state API while explicit move mode is unlocked. A missing or
//! stopped SteamVR runtime is reported through [`VrOverlayStatus`] and never terminates the
//! application.

use fontdue::{Font, FontSettings, Metrics};
use minmaxxer_core::EngineSnapshot;
use openvr::{
    button_id, overlay::OverlayHandle, pose::Matrix3x4, tracked_device_index, ApplicationType,
    Context, Overlay, System, TrackedControllerRole, TrackedDeviceIndex, TrackingUniverseOrigin,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, future::pending, path::PathBuf, time::Duration};
use tauri::async_runtime::JoinHandle;
use tokio::{
    sync::{oneshot, watch},
    time::{sleep_until, Instant},
};

pub const HUD_TEXTURE_WIDTH: usize = 1024;
pub const HUD_TEXTURE_HEIGHT: usize = 512;

const HUD_BYTES_PER_PIXEL: usize = 4;
const MIN_SUBMIT_INTERVAL: Duration = Duration::from_millis(250);
const RECONNECT_INTERVAL: Duration = Duration::from_secs(5);
const CONTROLLER_IDLE_INTERVAL: Duration = Duration::from_millis(50);
const CONTROLLER_PLACING_INTERVAL: Duration = Duration::from_millis(16);

// openvr 0.9's safe-looking `create_overlay(&str, &str)` wrapper forwards `str::as_ptr()` to a C
// API without appending a terminator. These constants MUST retain their trailing NUL bytes.
const OVERLAY_KEY: &str = "app.minmaxxer.ecliptica.hud\0";
const OVERLAY_NAME: &str = "MINMAXXER Ecliptica HUD\0";

/// User-facing configuration for the native SteamVR overlay.
///
/// Coordinates are metres in HMD-local space: positive X is right, positive Y is up, and
/// negative Z is in front of the headset. `controller_grab_enabled` is an explicit unlock mode:
/// each right-grip press immediately attaches the existing panel pose to the right controller,
/// and release freezes that pose relative to the HMD while continuing to listen for another
/// right-grip grab. Turning the toggle off finalizes the current pose and locks the HUD there.
/// The grabbed pose is runtime-only and is cleared when the VR HUD is disabled or XYZ settings
/// change.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VrOverlaySettings {
    pub enabled: bool,
    pub width_m: f32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub opacity: f32,
    pub curvature: f32,
    pub rows: u8,
    pub controller_grab_enabled: bool,
    pub show_status: bool,
    pub show_encounter: bool,
    pub show_phase: bool,
    pub show_boss_number: bool,
    pub show_focus: bool,
    pub show_rolling_dps: bool,
    pub show_total_damage: bool,
    pub show_incoming: bool,
    pub show_players: bool,
    pub show_attacks: bool,
    pub show_recent_hits: bool,
    pub recent_hit_rows: u8,
    pub show_loadout: bool,
}

impl Default for VrOverlaySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            width_m: 0.86,
            x: 0.30,
            y: 0.08,
            z: -1.05,
            opacity: 1.0,
            curvature: 0.08,
            rows: 5,
            controller_grab_enabled: false,
            show_status: true,
            show_encounter: true,
            show_phase: true,
            show_boss_number: true,
            show_focus: true,
            show_rolling_dps: true,
            show_total_damage: true,
            show_incoming: true,
            show_players: true,
            show_attacks: true,
            show_recent_hits: true,
            recent_hit_rows: 5,
            show_loadout: false,
        }
    }
}

/// Public placement state suitable for settings UI and tray status indicators.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VrPlacementState {
    #[default]
    Disabled,
    Listening,
    Arming,
    Moving,
    Placed,
    Unavailable,
}

impl VrOverlaySettings {
    /// Returns values safe to pass to OpenVR. This also protects against non-finite values arriving
    /// from a hand-edited settings file or JavaScript bridge.
    pub fn sanitized(mut self) -> Self {
        self.width_m = finite_clamped(self.width_m, 0.10, 3.0, 0.86);
        self.x = finite_clamped(self.x, -5.0, 5.0, 0.30);
        self.y = finite_clamped(self.y, -5.0, 5.0, 0.08);
        self.z = finite_clamped(self.z, -10.0, -0.05, -1.05);
        self.opacity = finite_clamped(self.opacity, 0.0, 1.0, 1.0);
        self.curvature = finite_clamped(self.curvature, 0.0, 1.0, 0.08);
        self.rows = self.rows.clamp(1, 8);
        self.recent_hit_rows = self.recent_hit_rows.clamp(1, 8);
        self
    }
}

/// Observable lifecycle state for the VR overlay worker.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct VrOverlayStatus {
    /// Whether the current settings request the overlay.
    pub enabled: bool,
    /// Whether an OpenVR overlay context and handle currently exist.
    pub active: bool,
    /// Whether the handle has been shown after a successful texture upload.
    pub visible: bool,
    /// Becomes true after the installed SteamVR runtime has been found.
    pub runtime_available: bool,
    /// Number of successful raw texture submissions in this worker lifetime.
    pub frames_submitted: u64,
    /// Engine snapshot version represented by the last successful frame.
    pub snapshot_version: Option<u64>,
    /// Current deliberate controller-grab gesture phase.
    pub placement_state: VrPlacementState,
    /// True after a controller placement has produced an HMD-relative runtime transform.
    pub runtime_placement_active: bool,
    /// Human-readable gesture instruction or availability note.
    pub placement_note: Option<String>,
    /// Most recent initialization or submission error, cleared after recovery or disable.
    pub last_error: Option<String>,
}

/// Handle returned by [`spawn_vr_overlay`]. Dropping it requests an orderly shutdown.
pub struct VrOverlayHandle {
    pub status: watch::Receiver<VrOverlayStatus>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    _task: JoinHandle<()>,
}

impl VrOverlayHandle {
    pub fn subscribe_status(&self) -> watch::Receiver<VrOverlayStatus> {
        self.status.clone()
    }

    pub fn request_shutdown(&mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
    }
}

impl Drop for VrOverlayHandle {
    fn drop(&mut self) {
        self.request_shutdown();
    }
}

/// Starts the native overlay worker on Tauri's shared async runtime.
pub fn spawn_vr_overlay(
    snapshot_rx: watch::Receiver<EngineSnapshot>,
    settings_rx: watch::Receiver<VrOverlaySettings>,
) -> VrOverlayHandle {
    let initial_settings = settings_rx.borrow().sanitized();
    let mut initial_status = VrOverlayStatus {
        enabled: initial_settings.enabled,
        ..VrOverlayStatus::default()
    };
    let initial_placement_state =
        if initial_settings.enabled && initial_settings.controller_grab_enabled {
            VrPlacementState::Unavailable
        } else {
            VrPlacementState::Disabled
        };
    update_placement_status(
        &mut initial_status,
        initial_placement_state,
        false,
        false,
        None,
    );
    let (status_tx, status_rx) = watch::channel(initial_status);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let task = tauri::async_runtime::spawn(run_vr_overlay(
        snapshot_rx,
        settings_rx,
        status_tx,
        shutdown_rx,
    ));
    VrOverlayHandle {
        status: status_rx,
        shutdown_tx: Some(shutdown_tx),
        _task: task,
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct GripInput {
    right_grip: Option<bool>,
    controllers_available: bool,
    right_pose_available: bool,
}

#[derive(Debug, Clone, Copy)]
struct ControllerPoll {
    input: GripInput,
    right_index: Option<TrackedDeviceIndex>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum GrabPhase {
    #[default]
    Waiting,
    Moving,
    AwaitRelease,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GestureAction {
    None,
    BeginMoving,
    FinishMoving,
}

#[derive(Debug, Clone, Copy, Default)]
struct GrabGesture {
    phase: GrabPhase,
    right_was_pressed: bool,
}

impl GrabGesture {
    fn update(&mut self, input: GripInput, _elapsed: Duration) -> GestureAction {
        let right_pressed = input.right_grip == Some(true);
        let just_pressed = right_pressed && !self.right_was_pressed;
        let action = match self.phase {
            GrabPhase::Waiting if just_pressed && input.right_pose_available => {
                self.phase = GrabPhase::Moving;
                GestureAction::BeginMoving
            }
            // If tracking is briefly lost on release, keep the controller-relative transform
            // alive until a valid pose can be converted back to HMD space.
            GrabPhase::Moving if !right_pressed && input.right_pose_available => {
                self.phase = GrabPhase::Waiting;
                GestureAction::FinishMoving
            }
            GrabPhase::AwaitRelease if !right_pressed => {
                self.phase = GrabPhase::Waiting;
                GestureAction::None
            }
            _ => GestureAction::None,
        };
        self.right_was_pressed = right_pressed;
        action
    }

    fn placement_state(self, controllers_available: bool) -> VrPlacementState {
        match self.phase {
            GrabPhase::Waiting if controllers_available => VrPlacementState::Listening,
            GrabPhase::Waiting => VrPlacementState::Unavailable,
            GrabPhase::Moving => VrPlacementState::Moving,
            GrabPhase::AwaitRelease => VrPlacementState::Placed,
        }
    }

    fn is_moving(self) -> bool {
        self.phase == GrabPhase::Moving
    }

    fn abort_until_release(&mut self) {
        self.phase = GrabPhase::AwaitRelease;
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Runs the overlay state machine. This lower-level entry point is useful when the owner wants to
/// create or forward its own status channel.
pub async fn run_vr_overlay(
    mut snapshot_rx: watch::Receiver<EngineSnapshot>,
    mut settings_rx: watch::Receiver<VrOverlaySettings>,
    status_tx: watch::Sender<VrOverlayStatus>,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    let mut snapshot = snapshot_rx.borrow_and_update().clone();
    let mut settings = settings_rx.borrow_and_update().sanitized();
    let mut status = VrOverlayStatus {
        enabled: settings.enabled,
        ..VrOverlayStatus::default()
    };

    let mut session: Option<VrSession> = None;
    let mut dirty = settings.enabled;
    let mut last_submit: Option<Instant> = None;
    let mut reconnect_at = Instant::now();
    let mut next_controller_poll = Instant::now();
    let mut last_controller_poll = Instant::now();
    let mut gesture = GrabGesture::default();
    let mut placement_state = if settings.enabled && settings.controller_grab_enabled {
        VrPlacementState::Unavailable
    } else {
        VrPlacementState::Disabled
    };
    let mut runtime_transform: Option<RigidTransform> = None;
    let mut placement_error: Option<String> = None;
    update_placement_status(
        &mut status,
        placement_state,
        false,
        false,
        placement_error.as_deref(),
    );
    publish_status(&status_tx, &status);

    'worker: loop {
        if !settings.enabled {
            if let Some(mut old_session) = session.take() {
                old_session.hide();
            }
            dirty = false;
            gesture.reset();
            placement_state = VrPlacementState::Disabled;
            runtime_transform = None;
            placement_error = None;
            status.enabled = false;
            status.active = false;
            status.visible = false;
            status.snapshot_version = None;
            status.last_error = None;
            update_placement_status(&mut status, placement_state, false, false, None);
            publish_status(&status_tx, &status);
        } else if session.is_none() && Instant::now() >= reconnect_at {
            status.enabled = true;
            match VrSession::connect() {
                Ok(new_session) => {
                    status.runtime_available = true;
                    status.active = true;
                    status.visible = false;
                    status.last_error = None;
                    session = Some(new_session);
                    gesture.reset();
                    placement_state = if settings.controller_grab_enabled {
                        VrPlacementState::Listening
                    } else {
                        VrPlacementState::Disabled
                    };
                    next_controller_poll = Instant::now();
                    last_controller_poll = Instant::now();
                    dirty = true;
                }
                Err(error) => {
                    status.runtime_available = error.runtime_available;
                    status.active = false;
                    status.visible = false;
                    status.last_error = Some(error.message);
                    reconnect_at = Instant::now() + RECONNECT_INTERVAL;
                }
            }
            publish_status(&status_tx, &status);
        }

        if settings.enabled {
            if let Some(current_session) = session.as_mut() {
                if !settings.controller_grab_enabled {
                    if current_session.is_controller_placing() {
                        let fallback = effective_transform(&settings, runtime_transform);
                        match current_session.finish_controller_placement() {
                            Ok(transform) => {
                                runtime_transform = Some(transform);
                                placement_error = None;
                            }
                            Err(error) => {
                                let _ = current_session.cancel_controller_placement(fallback);
                                placement_error = Some(format!(
                                    "HUD move mode was locked, but the final controller pose could not be read; the prior position was restored: {error}"
                                ));
                            }
                        }
                    }
                    gesture.reset();
                    if placement_state != VrPlacementState::Disabled {
                        placement_state = VrPlacementState::Disabled;
                        dirty = true;
                    }
                    update_placement_status(
                        &mut status,
                        placement_state,
                        runtime_transform.is_some(),
                        false,
                        placement_error.as_deref(),
                    );
                } else if Instant::now() >= next_controller_poll {
                    let now = Instant::now();
                    let elapsed = now.duration_since(last_controller_poll);
                    last_controller_poll = now;
                    let poll = current_session.poll_controllers(gesture.is_moving());
                    let action = gesture.update(poll.input, elapsed);

                    match action {
                        GestureAction::BeginMoving => {
                            let current_transform =
                                effective_transform(&settings, runtime_transform);
                            let result = poll
                                .right_index
                                .ok_or_else(|| "Right controller is unavailable".to_owned())
                                .and_then(|right_index| {
                                    current_session
                                        .begin_controller_placement(right_index, current_transform)
                                });
                            match result {
                                Ok(()) => placement_error = None,
                                Err(error) => {
                                    gesture.abort_until_release();
                                    placement_error = Some(format!(
                                        "Controller placement failed; the prior HUD position was kept: {error}"
                                    ));
                                }
                            }
                        }
                        GestureAction::FinishMoving => {
                            match current_session.finish_controller_placement() {
                                Ok(transform) => {
                                    runtime_transform = Some(transform);
                                    placement_error = None;
                                }
                                Err(error) => {
                                    let fallback =
                                        effective_transform(&settings, runtime_transform);
                                    let _ = current_session.cancel_controller_placement(fallback);
                                    placement_error = Some(format!(
                                        "Controller placement failed; the prior HUD position was restored: {error}"
                                    ));
                                }
                            }
                        }
                        GestureAction::None => {
                            if gesture.phase == GrabPhase::Waiting {
                                placement_error = None;
                            }
                        }
                    }

                    let next_state = if placement_error.is_some() {
                        VrPlacementState::Unavailable
                    } else {
                        gesture.placement_state(poll.input.controllers_available)
                    };
                    if next_state != placement_state {
                        placement_state = next_state;
                        dirty = true;
                    }
                    update_placement_status(
                        &mut status,
                        placement_state,
                        runtime_transform.is_some() && !gesture.is_moving(),
                        poll.input.right_pose_available,
                        placement_error.as_deref(),
                    );
                    next_controller_poll = now
                        + if gesture.is_moving() {
                            CONTROLLER_PLACING_INTERVAL
                        } else {
                            CONTROLLER_IDLE_INTERVAL
                        };
                    publish_status(&status_tx, &status);
                }
            } else if settings.controller_grab_enabled {
                placement_state = VrPlacementState::Unavailable;
                update_placement_status(
                    &mut status,
                    placement_state,
                    runtime_transform.is_some(),
                    false,
                    Some("Waiting for SteamVR before controller placement can start"),
                );
                publish_status(&status_tx, &status);
            }
        }

        let submit_ready = settings.enabled
            && session.is_some()
            && dirty
            && last_submit
                .map(|last| Instant::now().duration_since(last) >= MIN_SUBMIT_INTERVAL)
                .unwrap_or(true);

        if submit_ready {
            let current_session = session.as_mut().expect("session checked above");
            let transform = effective_transform(&settings, runtime_transform);
            match current_session.submit(&snapshot, &settings, placement_state, transform) {
                Ok(()) => {
                    dirty = false;
                    last_submit = Some(Instant::now());
                    status.active = true;
                    status.visible = true;
                    status.frames_submitted = status.frames_submitted.saturating_add(1);
                    status.snapshot_version = Some(snapshot.version);
                    status.last_error = placement_error.clone();
                }
                Err(error) => {
                    current_session.hide();
                    session = None;
                    status.active = false;
                    status.visible = false;
                    status.last_error = Some(error);
                    gesture.reset();
                    placement_state = if settings.controller_grab_enabled {
                        VrPlacementState::Unavailable
                    } else {
                        VrPlacementState::Disabled
                    };
                    reconnect_at = Instant::now() + RECONNECT_INTERVAL;
                }
            }
            publish_status(&status_tx, &status);
        }

        let now = Instant::now();
        let submit_or_reconnect_at = if !settings.enabled {
            None
        } else if session.is_none() {
            Some(reconnect_at)
        } else if dirty {
            Some(
                last_submit
                    .map(|last| last + MIN_SUBMIT_INTERVAL)
                    .unwrap_or(now),
            )
        } else {
            None
        };
        let controller_at =
            if settings.enabled && session.is_some() && settings.controller_grab_enabled {
                Some(next_controller_poll)
            } else {
                None
            };
        let wake_at = earliest_deadline(submit_or_reconnect_at, controller_at);

        tokio::select! {
            biased;
            _ = &mut shutdown_rx => break 'worker,
            changed = settings_rx.changed() => {
                if changed.is_err() {
                    break 'worker;
                }
                let next = settings_rx.borrow_and_update().sanitized();
                if next != settings {
                    let enabling = !settings.enabled && next.enabled;
                    let placement_coordinates_changed = next.x != settings.x
                        || next.y != settings.y
                        || next.z != settings.z;
                    let grab_setting_changed =
                        next.controller_grab_enabled != settings.controller_grab_enabled;
                    if placement_coordinates_changed {
                        runtime_transform = None;
                        placement_error = None;
                        gesture.reset();
                        if let Some(current_session) = session.as_mut() {
                            let _ = current_session
                                .cancel_controller_placement(configured_transform(&next));
                        }
                    }
                    settings = next;
                    status.enabled = settings.enabled;
                    dirty = settings.enabled;
                    if enabling {
                        reconnect_at = Instant::now();
                    }
                    if enabling || grab_setting_changed || placement_coordinates_changed {
                        next_controller_poll = Instant::now();
                        last_controller_poll = Instant::now();
                    }
                }
            }
            changed = snapshot_rx.changed() => {
                if changed.is_err() {
                    break 'worker;
                }
                let next = snapshot_rx.borrow_and_update().clone();
                if next != snapshot {
                    snapshot = next;
                    dirty = settings.enabled;
                }
            }
            _ = wait_until_some(wake_at) => {}
        }
    }

    if let Some(mut active_session) = session {
        active_session.hide();
    }
    status.enabled = false;
    status.active = false;
    status.visible = false;
    status.snapshot_version = None;
    update_placement_status(&mut status, VrPlacementState::Disabled, false, false, None);
    publish_status(&status_tx, &status);
}

fn earliest_deadline(left: Option<Instant>, right: Option<Instant>) -> Option<Instant> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(deadline), None) | (None, Some(deadline)) => Some(deadline),
        (None, None) => None,
    }
}

fn update_placement_status(
    status: &mut VrOverlayStatus,
    state: VrPlacementState,
    runtime_placement_active: bool,
    right_pose_available: bool,
    error: Option<&str>,
) {
    let note = error.or(match state {
        VrPlacementState::Disabled if status.enabled => Some("HUD locked in place"),
        VrPlacementState::Disabled => None,
        VrPlacementState::Listening => {
            Some("HUD unlocked; press and hold the right grip to move it, then release to place")
        }
        VrPlacementState::Arming if right_pose_available => {
            Some("HUD unlocked; waiting for the next right-grip press")
        }
        VrPlacementState::Arming => Some("HUD unlocked; waiting for right-controller tracking"),
        VrPlacementState::Moving if right_pose_available => {
            Some("Move the right controller; release its grip to place the HUD")
        }
        VrPlacementState::Moving => {
            Some("HUD move is active; waiting for right-controller tracking")
        }
        VrPlacementState::Placed => Some("Release the right grip before grabbing the HUD again"),
        VrPlacementState::Unavailable => {
            Some("HUD unlocked; waiting for the right SteamVR controller")
        }
    });
    if status.placement_state != state
        || status.runtime_placement_active != runtime_placement_active
        || status.placement_note.as_deref() != note
    {
        status.placement_state = state;
        status.runtime_placement_active = runtime_placement_active;
        status.placement_note = note.map(str::to_owned);
    }
}

async fn wait_until_some(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => sleep_until(deadline).await,
        None => pending::<()>().await,
    }
}

fn publish_status(status_tx: &watch::Sender<VrOverlayStatus>, status: &VrOverlayStatus) {
    if *status_tx.borrow() != *status {
        status_tx.send_replace(status.clone());
    }
}

struct SessionError {
    message: String,
    runtime_available: bool,
}

struct VrSession {
    // Declared after the OpenVR interfaces so it is dropped last.
    overlay: Overlay,
    system: System,
    handle: OverlayHandle,
    renderer: HudRenderer,
    controller_placement: Option<ActiveControllerPlacement>,
    applied: AppliedOverlayState,
    visible: bool,
    visibility_dirty: bool,
    _context: Context,
}

#[derive(Debug, Clone, Copy)]
struct ActiveControllerPlacement {
    right_index: TrackedDeviceIndex,
    controller_to_overlay: RigidTransform,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransformKind {
    Hmd,
    Controller(u32),
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct AppliedTransform {
    kind: TransformKind,
    transform: RigidTransform,
}

/// Last compositor properties that were successfully applied. Raw texture submissions do not
/// need to reassert any of these values, and repeatedly doing so can produce visible SteamVR
/// flicker. A fresh session starts empty and records a property only after its FFI call succeeds.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct AppliedOverlayState {
    width_m: Option<f32>,
    opacity: Option<f32>,
    curvature: Option<f32>,
    transform: Option<AppliedTransform>,
}

impl AppliedOverlayState {
    fn needs_width(self, value: f32) -> bool {
        self.width_m != Some(value)
    }

    fn needs_opacity(self, value: f32) -> bool {
        self.opacity != Some(value)
    }

    fn needs_curvature(self, value: f32) -> bool {
        self.curvature != Some(value)
    }

    fn transform_change(self, next: AppliedTransform) -> (bool, bool) {
        (
            self.transform != Some(next),
            self.transform
                .is_some_and(|current| current.kind != next.kind),
        )
    }
}

impl VrSession {
    fn connect() -> Result<Self, SessionError> {
        let runtime_available = openvr::is_runtime_installed();
        if !runtime_available {
            return Err(SessionError {
                message: "SteamVR is not installed; the VR HUD remains disabled".to_owned(),
                runtime_available: false,
            });
        }

        let renderer = HudRenderer::new().map_err(|message| SessionError {
            message,
            runtime_available: true,
        })?;

        // SAFETY: this worker owns the only OpenVR context used by this module. `VrSession` keeps
        // the context alive longer than its interfaces, and dropping the session releases it.
        // catch_unwind converts openvr 0.9's "already initialized" panic into app-visible status.
        let context =
            match std::panic::catch_unwind(|| unsafe { openvr::init(ApplicationType::Overlay) }) {
                Ok(Ok(context)) => context,
                Ok(Err(error)) => {
                    return Err(SessionError {
                        message: format!("SteamVR overlay initialization failed: {error}"),
                        runtime_available: true,
                    });
                }
                Err(_) => {
                    return Err(SessionError {
                        message: "SteamVR is already initialized by another in-process client"
                            .to_owned(),
                        runtime_available: true,
                    });
                }
            };

        let system = context.system().map_err(|error| SessionError {
            message: format!("SteamVR tracking interface is unavailable: {error}"),
            runtime_available: true,
        })?;
        let mut overlay = context.overlay().map_err(|error| SessionError {
            message: format!("SteamVR overlay interface is unavailable: {error}"),
            runtime_available: true,
        })?;
        let handle = overlay
            .create_overlay(OVERLAY_KEY, OVERLAY_NAME)
            .map_err(|error| SessionError {
                message: format!("SteamVR could not create the HUD overlay: {error:?}"),
                runtime_available: true,
            })?;
        // The sort order is best-effort: a runtime may reject it while still accepting the HUD.
        let _ = overlay.set_sort_order(handle, 100);

        Ok(Self {
            overlay,
            system,
            handle,
            renderer,
            controller_placement: None,
            applied: AppliedOverlayState::default(),
            visible: false,
            visibility_dirty: false,
            _context: context,
        })
    }

    fn submit(
        &mut self,
        snapshot: &EngineSnapshot,
        settings: &VrOverlaySettings,
        placement_state: VrPlacementState,
        hmd_transform: RigidTransform,
    ) -> Result<(), String> {
        let settings = settings.sanitized();
        if self.applied.needs_width(settings.width_m) {
            self.overlay
                .set_width(self.handle, settings.width_m)
                .map_err(|error| format!("SteamVR rejected HUD width: {error:?}"))?;
            self.applied.width_m = Some(settings.width_m);
        }
        if self.applied.needs_opacity(settings.opacity) {
            self.overlay
                .set_opacity(self.handle, settings.opacity)
                .map_err(|error| format!("SteamVR rejected HUD opacity: {error:?}"))?;
            self.applied.opacity = Some(settings.opacity);
        }
        if self.applied.needs_curvature(settings.curvature) {
            self.overlay
                .set_curvature(self.handle, settings.curvature)
                .map_err(|error| format!("SteamVR rejected HUD curvature: {error:?}"))?;
            self.applied.curvature = Some(settings.curvature);
        }

        if self.controller_placement.is_none() {
            self.set_hmd_transform(hmd_transform)?;
        }

        let pixels = self
            .renderer
            .render_with_placement(snapshot, &settings, placement_state);
        self.overlay
            .set_raw_data(
                self.handle,
                pixels,
                HUD_TEXTURE_WIDTH,
                HUD_TEXTURE_HEIGHT,
                HUD_BYTES_PER_PIXEL,
            )
            .map_err(|error| format!("SteamVR texture upload failed: {error:?}"))?;

        self.ensure_visible()?;
        Ok(())
    }

    fn poll_controllers(&self, placement_active: bool) -> ControllerPoll {
        let right_index = self
            .controller_placement
            .map(|placement| placement.right_index)
            .or_else(|| {
                self.system
                    .tracked_device_index_for_controller_role(TrackedControllerRole::RightHand)
            });
        let right_grip = right_index
            .and_then(|index| self.system.controller_state(index))
            .map(|state| grip_pressed(state.button_pressed));
        let controllers_available = right_grip.is_some();
        // SteamVR itself updates a tracked-device-relative overlay while it is moving. A pose read
        // is only needed on the initial right-grip press or to convert the released pose back into
        // HMD space.
        let pose_needed = (!placement_active && right_grip == Some(true))
            || (placement_active && right_grip == Some(false));
        let right_pose_available = if pose_needed {
            right_index
                .and_then(|index| self.hmd_and_controller_poses(index))
                .is_some()
        } else {
            placement_active && right_grip.is_some()
        };
        ControllerPoll {
            input: GripInput {
                right_grip,
                controllers_available,
                right_pose_available,
            },
            right_index,
        }
    }

    fn begin_controller_placement(
        &mut self,
        right_index: TrackedDeviceIndex,
        current_hmd_transform: RigidTransform,
    ) -> Result<(), String> {
        let (world_from_hmd, world_from_controller) = self
            .hmd_and_controller_poses(right_index)
            .ok_or_else(|| "HMD or right-controller pose is not currently valid".to_owned())?;
        let world_from_overlay = world_from_hmd.multiply(current_hmd_transform);
        let controller_to_overlay = world_from_controller
            .inverse_rigid()
            .multiply(world_from_overlay);
        self.set_controller_transform(right_index, controller_to_overlay)?;
        self.controller_placement = Some(ActiveControllerPlacement {
            right_index,
            controller_to_overlay,
        });
        self.system.trigger_haptic_pulse(right_index, 0, 900);
        Ok(())
    }

    fn finish_controller_placement(&mut self) -> Result<RigidTransform, String> {
        let placement = self
            .controller_placement
            .ok_or_else(|| "Controller placement was not active".to_owned())?;
        let (world_from_hmd, world_from_controller) = self
            .hmd_and_controller_poses(placement.right_index)
            .ok_or_else(|| "HMD or right-controller pose was lost before release".to_owned())?;
        let world_from_overlay = world_from_controller.multiply(placement.controller_to_overlay);
        let hmd_to_overlay = world_from_hmd.inverse_rigid().multiply(world_from_overlay);
        self.set_hmd_transform(hmd_to_overlay)?;
        self.controller_placement = None;
        self.system
            .trigger_haptic_pulse(placement.right_index, 0, 1_400);
        Ok(hmd_to_overlay)
    }

    fn cancel_controller_placement(&mut self, fallback: RigidTransform) -> Result<(), String> {
        let result = self.set_hmd_transform(fallback);
        self.controller_placement = None;
        result
    }

    fn is_controller_placing(&self) -> bool {
        self.controller_placement.is_some()
    }

    fn set_hmd_transform(&mut self, transform: RigidTransform) -> Result<(), String> {
        let next = AppliedTransform {
            kind: TransformKind::Hmd,
            transform,
        };
        let (changed, kind_changed) = self.applied.transform_change(next);
        if !changed {
            return Ok(());
        }
        let openvr_transform = transform.to_openvr();
        self.overlay
            .set_transform_tracked_device_relative(
                self.handle,
                tracked_device_index::HMD,
                &openvr_transform,
            )
            .map_err(|error| format!("SteamVR rejected HUD placement: {error:?}"))?;
        self.applied.transform = Some(next);
        self.reassert_visibility_after_transform_kind_change(kind_changed)
    }

    fn set_controller_transform(
        &mut self,
        right_index: TrackedDeviceIndex,
        transform: RigidTransform,
    ) -> Result<(), String> {
        let next = AppliedTransform {
            kind: TransformKind::Controller(right_index.0),
            transform,
        };
        let (changed, kind_changed) = self.applied.transform_change(next);
        if !changed {
            return Ok(());
        }
        let openvr_transform = transform.to_openvr();
        self.overlay
            .set_transform_tracked_device_relative(self.handle, right_index, &openvr_transform)
            .map_err(|error| {
                format!("SteamVR rejected controller-relative placement: {error:?}")
            })?;
        self.applied.transform = Some(next);
        self.reassert_visibility_after_transform_kind_change(kind_changed)
    }

    fn reassert_visibility_after_transform_kind_change(
        &mut self,
        kind_changed: bool,
    ) -> Result<(), String> {
        if kind_changed && self.visible {
            // VROS observed that switching between device-relative transform kinds can make an
            // otherwise-visible overlay disappear. Mark then immediately reconcile; if the call
            // fails the dirty bit remains set for the next submission.
            self.visibility_dirty = true;
            self.ensure_visible()?;
        }
        Ok(())
    }

    fn ensure_visible(&mut self) -> Result<(), String> {
        if !self.visible || self.visibility_dirty {
            self.overlay
                .set_visibility(self.handle, true)
                .map_err(|error| format!("SteamVR could not show the HUD: {error:?}"))?;
            self.visible = true;
            self.visibility_dirty = false;
        }
        Ok(())
    }

    fn hmd_and_controller_poses(
        &self,
        controller: TrackedDeviceIndex,
    ) -> Option<(RigidTransform, RigidTransform)> {
        let poses = self
            .system
            .device_to_absolute_tracking_pose(TrackingUniverseOrigin::Standing, 0.0);
        let hmd = poses.get(tracked_device_index::HMD.0 as usize)?;
        let controller = poses.get(controller.0 as usize)?;
        if !hmd.pose_is_valid()
            || !hmd.device_is_connected()
            || !controller.pose_is_valid()
            || !controller.device_is_connected()
        {
            return None;
        }
        Some((
            RigidTransform::from_rows(*hmd.device_to_absolute_tracking()),
            RigidTransform::from_rows(*controller.device_to_absolute_tracking()),
        ))
    }

    fn hide(&mut self) {
        if self.visible {
            let _ = self.overlay.set_visibility(self.handle, false);
            self.visible = false;
        }
        self.visibility_dirty = false;
    }
}

impl Drop for VrSession {
    fn drop(&mut self) {
        self.hide();
    }
}

fn grip_pressed(buttons: u64) -> bool {
    let mask = 1_u64 << button_id::GRIP;
    buttons & mask != 0
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct RigidTransform([[f32; 4]; 3]);

impl RigidTransform {
    fn from_rows(rows: [[f32; 4]; 3]) -> Self {
        Self(rows)
    }

    fn to_openvr(self) -> Matrix3x4 {
        Matrix3x4(self.0)
    }

    fn multiply(self, right: Self) -> Self {
        let mut result = [[0.0; 4]; 3];
        for (row, result_row) in result.iter_mut().enumerate() {
            for (column, value) in result_row.iter_mut().take(3).enumerate() {
                *value = (0..3)
                    .map(|middle| self.0[row][middle] * right.0[middle][column])
                    .sum();
            }
            result_row[3] = self.0[row][3]
                + (0..3)
                    .map(|middle| self.0[row][middle] * right.0[middle][3])
                    .sum::<f32>();
        }
        Self(result)
    }

    fn inverse_rigid(self) -> Self {
        let mut inverse = [[0.0; 4]; 3];
        for (row, inverse_row) in inverse.iter_mut().enumerate() {
            for (column, value) in inverse_row.iter_mut().take(3).enumerate() {
                *value = self.0[column][row];
            }
            inverse_row[3] = -(0..3)
                .map(|column| inverse_row[column] * self.0[column][3])
                .sum::<f32>();
        }
        Self(inverse)
    }
}

fn configured_transform(settings: &VrOverlaySettings) -> RigidTransform {
    RigidTransform::from_rows([
        [1.0, 0.0, 0.0, settings.x],
        [0.0, 1.0, 0.0, settings.y],
        [0.0, 0.0, 1.0, settings.z],
    ])
}

fn effective_transform(
    settings: &VrOverlaySettings,
    runtime_transform: Option<RigidTransform>,
) -> RigidTransform {
    runtime_transform.unwrap_or_else(|| configured_transform(settings))
}

fn finite_clamped(value: f32, minimum: f32, maximum: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(minimum, maximum)
    } else {
        fallback
    }
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
struct GlyphKey {
    character: char,
    pixels: u16,
}

struct CachedGlyph {
    metrics: Metrics,
    coverage: Vec<u8>,
}

/// CPU-only HUD renderer. It has no OpenVR dependency at runtime and can also back previews/tests.
/// The 2 MiB output buffer is allocated once and reused between frames.
pub struct HudRenderer {
    font: Font,
    glyphs: HashMap<GlyphKey, CachedGlyph>,
    pixels: Vec<u8>,
}

impl HudRenderer {
    /// Loads a bold system UI font when available. Portable fallbacks keep renderer tests and
    /// development builds usable on non-Windows hosts.
    pub fn new() -> Result<Self, String> {
        let font = load_ui_font()?;
        Ok(Self {
            font,
            glyphs: HashMap::new(),
            pixels: vec![0; HUD_TEXTURE_WIDTH * HUD_TEXTURE_HEIGHT * HUD_BYTES_PER_PIXEL],
        })
    }

    #[cfg(test)]
    pub fn width(&self) -> usize {
        HUD_TEXTURE_WIDTH
    }

    #[cfg(test)]
    pub fn height(&self) -> usize {
        HUD_TEXTURE_HEIGHT
    }

    #[cfg(test)]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    #[cfg(test)]
    /// Renders a complete straight-alpha RGBA8 frame and returns the renderer-owned pixel slice.
    pub fn render(&mut self, snapshot: &EngineSnapshot, settings: &VrOverlaySettings) -> &[u8] {
        self.render_with_placement(snapshot, settings, VrPlacementState::Disabled)
    }

    /// Renders the frame with native-controller placement feedback. OpenVR calls are not required.
    pub fn render_with_placement(
        &mut self,
        snapshot: &EngineSnapshot,
        settings: &VrOverlaySettings,
        placement_state: VrPlacementState,
    ) -> &[u8] {
        let settings = settings.sanitized();
        if self.glyphs.len() > 1024 {
            self.glyphs.clear();
        }
        self.pixels.fill(0);

        let Self {
            font,
            glyphs,
            pixels,
        } = self;
        render_frame(font, glyphs, pixels, snapshot, &settings, placement_state);
        pixels
    }
}

fn load_ui_font() -> Result<Font, String> {
    let mut candidates = Vec::new();
    if let Some(windows_dir) = std::env::var_os("WINDIR") {
        let fonts = PathBuf::from(windows_dir).join("Fonts");
        candidates.push(fonts.join("segoeuib.ttf"));
        candidates.push(fonts.join("arialbd.ttf"));
        candidates.push(fonts.join("segoeui.ttf"));
        candidates.push(fonts.join("SegoeUI.ttf"));
    }
    candidates.push(PathBuf::from(r"C:\Windows\Fonts\segoeuib.ttf"));
    candidates.push(PathBuf::from(r"C:\Windows\Fonts\arialbd.ttf"));
    candidates.push(PathBuf::from(r"C:\Windows\Fonts\segoeui.ttf"));
    candidates.push(PathBuf::from(
        "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
    ));
    candidates.push(PathBuf::from(
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ));
    candidates.push(PathBuf::from(
        "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
    ));
    candidates.push(PathBuf::from(
        "/System/Library/Fonts/Supplemental/Arial.ttf",
    ));

    let mut failures = Vec::new();
    for path in candidates {
        if let Ok(bytes) = fs::read(&path) {
            match Font::from_bytes(bytes, FontSettings::default()) {
                Ok(font) => return Ok(font),
                Err(error) => failures.push(format!("{} ({error})", path.display())),
            }
        }
    }
    let detail = if failures.is_empty() {
        "no supported UI font file was found".to_owned()
    } else {
        failures.join(", ")
    };
    Err(format!(
        "Could not load a system UI font for the VR HUD: {detail}"
    ))
}

#[derive(Clone, Copy)]
struct Color(u8, u8, u8, u8);

const TRANSPARENT: Color = Color(0, 0, 0, 0);
const PANEL: Color = Color(8, 13, 25, 232);
const PANEL_ALT: Color = Color(16, 25, 43, 178);
const CARD: Color = Color(23, 35, 58, 196);
const WHITE: Color = Color(244, 248, 255, 255);
const MUTED: Color = Color(157, 171, 194, 255);
const DIM: Color = Color(102, 118, 145, 255);
const CYAN: Color = Color(63, 220, 255, 255);
const VIOLET: Color = Color(177, 123, 255, 255);
const ORANGE: Color = Color(255, 170, 78, 255);
const RED: Color = Color(255, 101, 120, 255);
const GREEN: Color = Color(103, 232, 166, 255);
const UNAVAILABLE_TELEMETRY: [(&str, &str); 4] = [
    ("HP", "NOT LOGGED"),
    ("HEALING", "NOT LOGGED"),
    ("GEMS", "NOT LOGGED"),
    ("SONG", "NOT LOGGED"),
];

fn render_frame(
    font: &Font,
    glyphs: &mut HashMap<GlyphKey, CachedGlyph>,
    pixels: &mut [u8],
    snapshot: &EngineSnapshot,
    settings: &VrOverlaySettings,
    placement_state: VrPlacementState,
) {
    debug_assert_eq!(
        pixels.len(),
        HUD_TEXTURE_WIDTH * HUD_TEXTURE_HEIGHT * HUD_BYTES_PER_PIXEL
    );
    fill_rounded_rect(pixels, 8, 8, 1008, 496, 22, PANEL);
    fill_rounded_rect(pixels, 24, 23, 8, 48, 4, CYAN);
    draw_text(font, glyphs, pixels, "MINMAXXER", 44, 64, 36, WHITE, 340);
    draw_text(
        font,
        glyphs,
        pixels,
        "ECLIPTICA LIVE",
        304,
        60,
        19,
        CYAN,
        250,
    );

    if settings.show_status {
        let indicator = if snapshot.connected { GREEN } else { ORANGE };
        fill_circle(pixels, 982, 42, 7, indicator);
        let connection = if snapshot.connected {
            "LOG LIVE"
        } else {
            "WAITING"
        };
        draw_text_right(
            font, glyphs, pixels, connection, 963, 49, 15, indicator, 150,
        );
    }
    if let Some((placement, color)) = placement_header(placement_state) {
        draw_text_right(font, glyphs, pixels, placement, 963, 70, 14, color, 300);
    }

    let context = native_hud_context(snapshot, settings);
    draw_text(font, glyphs, pixels, &context.line, 44, 96, 19, WHITE, 920);

    fill_rounded_rect(pixels, 24, 105, 976, 35, 8, Color(23, 31, 49, 230));
    fill_rounded_rect(pixels, 24, 105, 6, 35, 3, ORANGE);
    draw_text(
        font,
        glyphs,
        pixels,
        &context.focus,
        41,
        130,
        19,
        if snapshot.focus.is_some() {
            ORANGE
        } else {
            MUTED
        },
        940,
    );

    draw_dps_panel(font, glyphs, pixels, snapshot, settings, 24, 150, 310, 288);
    draw_outgoing_feed(font, glyphs, pixels, snapshot, settings, 346, 150, 326, 288);
    draw_incoming_feed(font, glyphs, pixels, snapshot, settings, 684, 150, 316, 288);
    draw_unavailable_strip(font, glyphs, pixels);
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeHudContext {
    line: String,
    focus: String,
}

fn native_hud_context(snapshot: &EngineSnapshot, settings: &VrOverlaySettings) -> NativeHudContext {
    let mut parts = Vec::with_capacity(4);
    if settings.show_phase {
        let phase = snapshot
            .run_context
            .phase_name
            .as_deref()
            .unwrap_or("UNKNOWN");
        let value = snapshot
            .run_context
            .progress
            .map(|progress| format!(" {progress:.0}%", progress = progress * 100.0))
            .unwrap_or_default();
        parts.push(format!("PHASE {phase}{value}"));
    }
    if settings.show_boss_number {
        let round = snapshot
            .run_context
            .boss_number
            .map(|number| {
                let inferred = if snapshot.run_context.boss_number_inferred {
                    "~"
                } else {
                    ""
                };
                format!("ROUND {inferred}{number:02}/13")
            })
            .unwrap_or_else(|| "ROUND --/13".to_owned());
        parts.push(round);
    }
    if settings.show_encounter {
        let stage = snapshot
            .stage
            .as_deref()
            .map(friendly_entity_label)
            .unwrap_or_else(|| "NOT STARTED".to_owned());
        parts.push(format!("STAGE {stage}"));
        let boss = if snapshot.encounter.kind == "boss" && !snapshot.encounter.name.is_empty() {
            friendly_entity_label(&snapshot.encounter.name)
        } else {
            "NOT STARTED".to_owned()
        };
        parts.push(format!("BOSS {boss}"));
    }
    if parts.is_empty() {
        parts.push("ECLIPTICA RUN CONTEXT HIDDEN".to_owned());
    }

    let active_boss = snapshot.encounter.active && snapshot.encounter.kind == "boss";
    let focus = if !settings.show_focus {
        "BOSS TARGET? HIDDEN".to_owned()
    } else if !active_boss {
        "BOSS TARGET? NO ACTIVE BOSS".to_owned()
    } else if let Some(focus) = snapshot.focus.as_ref() {
        format!(
            "BOSS TARGET? {}  |  {} CONFIDENCE  |  {} OLD",
            focus.player.to_ascii_uppercase(),
            focus.confidence.to_ascii_uppercase(),
            format_age(focus.age_seconds)
        )
    } else {
        "BOSS TARGET? ACQUIRING OWNERSHIP SIGNAL".to_owned()
    };
    NativeHudContext {
        line: parts.join("  |  "),
        focus,
    }
}

fn placement_header(state: VrPlacementState) -> Option<(&'static str, Color)> {
    match state {
        VrPlacementState::Listening => Some(("MOVE UNLOCKED - RIGHT GRIP TO GRAB", CYAN)),
        VrPlacementState::Moving => Some(("MOVING - RELEASE RIGHT GRIP TO PLACE", GREEN)),
        VrPlacementState::Unavailable => {
            Some(("MOVE UNLOCKED - RIGHT CONTROLLER UNAVAILABLE", ORANGE))
        }
        VrPlacementState::Arming => Some(("MOVE UNLOCKED - RIGHT GRIP TO GRAB", ORANGE)),
        VrPlacementState::Placed => Some(("RELEASE RIGHT GRIP", CYAN)),
        VrPlacementState::Disabled => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_dps_panel(
    font: &Font,
    glyphs: &mut HashMap<GlyphKey, CachedGlyph>,
    pixels: &mut [u8],
    snapshot: &EngineSnapshot,
    settings: &VrOverlaySettings,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) {
    fill_rounded_rect(pixels, x, y, width, height, 13, PANEL_ALT);
    let segment = if snapshot.encounter.kind == "boss" {
        "BOSS FIGHT DPS"
    } else {
        "PRE-BOSS DPS"
    };
    draw_text(
        font,
        glyphs,
        pixels,
        segment,
        x + 15,
        y + 27,
        18,
        CYAN,
        width - 145,
    );
    let current_dps = if settings.show_rolling_dps {
        format!("{} /s", compact_number(snapshot.outgoing.rolling_5s))
    } else {
        "HIDDEN".to_owned()
    };
    draw_text_right(
        font,
        glyphs,
        pixels,
        &current_dps,
        x + width - 14,
        y + 32,
        25,
        WHITE,
        130,
    );

    let graph_x = x + 15;
    let graph_y = y + 50;
    let graph_width = width - 30;
    let graph_height = 153;
    fill_rounded_rect(
        pixels,
        graph_x,
        graph_y,
        graph_width,
        graph_height,
        7,
        Color(6, 11, 22, 220),
    );
    if settings.show_rolling_dps {
        draw_dps_graph(
            font,
            glyphs,
            pixels,
            snapshot,
            graph_x,
            graph_y,
            graph_width,
            graph_height,
        );
    } else {
        draw_text(
            font,
            glyphs,
            pixels,
            "DPS GRAPH HIDDEN",
            graph_x + 18,
            graph_y + 84,
            18,
            DIM,
            graph_width - 36,
        );
    }

    draw_text(
        font,
        glyphs,
        pixels,
        "SEGMENT DAMAGE",
        x + 15,
        y + 229,
        15,
        MUTED,
        width - 30,
    );
    let total = if settings.show_total_damage {
        compact_number(snapshot.outgoing.total)
    } else {
        "HIDDEN".to_owned()
    };
    draw_text(
        font,
        glyphs,
        pixels,
        &total,
        x + 15,
        y + 274,
        31,
        VIOLET,
        width - 30,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_dps_graph(
    font: &Font,
    glyphs: &mut HashMap<GlyphKey, CachedGlyph>,
    pixels: &mut [u8],
    snapshot: &EngineSnapshot,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) {
    let maximum = snapshot
        .timeline
        .iter()
        .map(|point| point.rolling_dps)
        .filter(|value| value.is_finite() && *value >= 0.0)
        .fold(snapshot.outgoing.rolling_5s.max(0.0), f64::max);
    let count = snapshot
        .timeline
        .iter()
        .filter(|point| point.rolling_dps.is_finite() && point.rolling_dps >= 0.0)
        .count();

    for grid in 1..4 {
        let grid_y = y + grid * height / 4;
        draw_line(
            pixels,
            x + 8,
            grid_y,
            x + width - 8,
            grid_y,
            Color(49, 65, 91, 95),
            1,
        );
    }
    if count < 2 || maximum <= 0.0 {
        draw_text(
            font,
            glyphs,
            pixels,
            "WAITING FOR SEGMENT DAMAGE",
            x + 14,
            y + height / 2 + 7,
            16,
            DIM,
            width - 28,
        );
        return;
    }

    let plot_left = x + 8;
    let plot_right = x + width - 8;
    let plot_top = y + 12;
    let plot_bottom = y + height - 12;
    let mut previous = None;
    for (visible_index, point) in snapshot
        .timeline
        .iter()
        .filter(|point| point.rolling_dps.is_finite() && point.rolling_dps >= 0.0)
        .enumerate()
    {
        let fraction_x = visible_index as f64 / (count.saturating_sub(1)) as f64;
        let fraction_y = (point.rolling_dps / maximum).clamp(0.0, 1.0);
        let point_x = plot_left + ((plot_right - plot_left) as f64 * fraction_x).round() as i32;
        let point_y = plot_bottom - ((plot_bottom - plot_top) as f64 * fraction_y).round() as i32;
        if let Some((previous_x, previous_y)) = previous {
            draw_line(pixels, previous_x, previous_y, point_x, point_y, CYAN, 2);
        }
        previous = Some((point_x, point_y));
    }
    draw_text(
        font,
        glyphs,
        pixels,
        &format!("PEAK {}", compact_number(maximum)),
        x + 12,
        y + 19,
        13,
        MUTED,
        width - 24,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_outgoing_feed(
    font: &Font,
    glyphs: &mut HashMap<GlyphKey, CachedGlyph>,
    pixels: &mut [u8],
    snapshot: &EngineSnapshot,
    settings: &VrOverlaySettings,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) {
    fill_rounded_rect(pixels, x, y, width, height, 13, PANEL_ALT);
    draw_text(
        font,
        glyphs,
        pixels,
        "LAST 5 LOCAL DAMAGE",
        x + 14,
        y + 27,
        18,
        WHITE,
        width - 135,
    );
    draw_text_right(
        font,
        glyphs,
        pixels,
        "CATEGORY ONLY",
        x + width - 13,
        y + 25,
        13,
        ORANGE,
        122,
    );
    if !settings.show_recent_hits {
        draw_text(
            font,
            glyphs,
            pixels,
            "OUTGOING FEED HIDDEN",
            x + 15,
            y + 73,
            18,
            DIM,
            width - 30,
        );
        return;
    }

    let hits = recent_local_hits(snapshot, settings.recent_hit_rows.min(5) as usize);
    if hits.is_empty() {
        draw_text(
            font,
            glyphs,
            pixels,
            "WAITING FOR LOCAL DAMAGE...",
            x + 15,
            y + 73,
            18,
            DIM,
            width - 30,
        );
        return;
    }

    let mut row_y = y + 42;
    for (index, hit) in hits.into_iter().enumerate() {
        if index % 2 == 0 {
            fill_rounded_rect(
                pixels,
                x + 8,
                row_y,
                width - 16,
                45,
                6,
                Color(27, 39, 62, 150),
            );
        }
        draw_text(
            font,
            glyphs,
            pixels,
            &compact_number(hit.amount),
            x + 14,
            row_y + 31,
            25,
            ORANGE,
            105,
        );
        let category = if hit.damage_type.eq_ignore_ascii_case("non-strike")
            || hit.damage_type.eq_ignore_ascii_case("non_strike")
        {
            "NON-STRIKE CAT."
        } else if hit.damage_type.eq_ignore_ascii_case("strike") {
            "STRIKE CAT."
        } else {
            "UNKNOWN CAT."
        };
        draw_text(
            font,
            glyphs,
            pixels,
            category,
            x + 119,
            row_y + 28,
            17,
            WHITE,
            width - 190,
        );
        draw_text_right(
            font,
            glyphs,
            pixels,
            &format_age(hit.age_seconds),
            x + width - 13,
            row_y + 27,
            14,
            MUTED,
            58,
        );
        row_y += 47;
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_incoming_feed(
    font: &Font,
    glyphs: &mut HashMap<GlyphKey, CachedGlyph>,
    pixels: &mut [u8],
    snapshot: &EngineSnapshot,
    settings: &VrOverlaySettings,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) {
    fill_rounded_rect(pixels, x, y, width, height, 13, PANEL_ALT);
    draw_text(
        font,
        glyphs,
        pixels,
        "RECENT DAMAGE TAKEN",
        x + 14,
        y + 27,
        18,
        WHITE,
        width - 142,
    );
    let total = if settings.show_incoming {
        compact_number(snapshot.incoming.total)
    } else {
        "HIDDEN".to_owned()
    };
    draw_text_right(
        font,
        glyphs,
        pixels,
        &format!("TOTAL {total}"),
        x + width - 13,
        y + 25,
        14,
        RED,
        135,
    );
    if !settings.show_incoming {
        draw_text(
            font,
            glyphs,
            pixels,
            "INCOMING FEED HIDDEN",
            x + 15,
            y + 73,
            18,
            DIM,
            width - 30,
        );
        return;
    }

    let incoming = recent_incoming_events(snapshot, 5);
    if incoming.is_empty() {
        draw_text(
            font,
            glyphs,
            pixels,
            "NO RECENT INCOMING DAMAGE",
            x + 15,
            y + 73,
            17,
            DIM,
            width - 30,
        );
        return;
    }

    let mut row_y = y + 42;
    for (index, event) in incoming.into_iter().enumerate() {
        if index % 2 == 0 {
            fill_rounded_rect(
                pixels,
                x + 8,
                row_y,
                width - 16,
                45,
                6,
                Color(49, 27, 43, 150),
            );
        }
        draw_text(
            font,
            glyphs,
            pixels,
            &compact_number(event.amount()),
            x + 14,
            row_y + 31,
            24,
            RED,
            70,
        );
        draw_text(
            font,
            glyphs,
            pixels,
            &friendly_incoming_source(event.source.as_deref().unwrap_or_default()),
            x + 87,
            row_y + 28,
            16,
            WHITE,
            width - 177,
        );
        draw_text_right(
            font,
            glyphs,
            pixels,
            &event.timestamp.format("%H:%M:%S").to_string(),
            x + width - 13,
            row_y + 27,
            12,
            MUTED,
            76,
        );
        row_y += 47;
    }
}

fn draw_unavailable_strip(
    font: &Font,
    glyphs: &mut HashMap<GlyphKey, CachedGlyph>,
    pixels: &mut [u8],
) {
    fill_rounded_rect(pixels, 24, 448, 976, 39, 9, Color(14, 21, 35, 230));
    let cell_width = 244;
    for (index, (label, value)) in UNAVAILABLE_TELEMETRY.into_iter().enumerate() {
        let cell_x = 24 + index as i32 * cell_width;
        if index > 0 {
            fill_rounded_rect(pixels, cell_x, 456, 1, 23, 0, Color(66, 79, 101, 150));
        }
        draw_text(font, glyphs, pixels, label, cell_x + 13, 474, 15, MUTED, 84);
        draw_text_right(
            font,
            glyphs,
            pixels,
            value,
            cell_x + cell_width - 13,
            474,
            16,
            ORANGE,
            135,
        );
    }
}

#[allow(dead_code)]
fn draw_metric_cards(
    font: &Font,
    glyphs: &mut HashMap<GlyphKey, CachedGlyph>,
    pixels: &mut [u8],
    metrics: &[(&str, String, Color)],
) {
    let gap = 14;
    let total_width = 962;
    let card_width = (total_width - gap * (metrics.len() as i32 - 1)) / metrics.len() as i32;
    for (index, (label, value, accent)) in metrics.iter().enumerate() {
        let x = 31 + index as i32 * (card_width + gap);
        fill_rounded_rect(pixels, x, 119, card_width, 101, 14, CARD);
        fill_rounded_rect(pixels, x + 15, 136, 5, 24, 2, *accent);
        draw_text(
            font,
            glyphs,
            pixels,
            label,
            x + 31,
            155,
            16,
            MUTED,
            card_width - 47,
        );
        draw_text(
            font,
            glyphs,
            pixels,
            value,
            x + 16,
            200,
            31,
            WHITE,
            card_width - 32,
        );
    }
}

#[allow(clippy::too_many_arguments, dead_code)]
fn draw_stats_column(
    font: &Font,
    glyphs: &mut HashMap<GlyphKey, CachedGlyph>,
    pixels: &mut [u8],
    snapshot: &EngineSnapshot,
    settings: &VrOverlaySettings,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) {
    fill_rounded_rect(pixels, x, y, width, height, 14, PANEL_ALT);
    let mut cursor_y = y + 25;
    let mut budget = settings.rows as usize;
    // Player summaries can contain DPS, total damage, and incoming DPS at once. Two text lines
    // keep all selected values readable in the split-column HUD instead of clipping the final
    // metric off the right edge.
    let row_height = 44;

    let mut players: Vec<_> = snapshot.players.iter().collect();
    players.sort_by(|left, right| {
        player_primary_metric(right, settings).total_cmp(&player_primary_metric(left, settings))
    });
    if settings.show_players && !players.is_empty() && budget > 0 {
        let heading = match (
            settings.show_rolling_dps,
            settings.show_total_damage,
            settings.show_incoming,
        ) {
            (true, false, false) => "PLAYERS · DPS",
            (false, true, false) => "PLAYERS · DAMAGE",
            (false, false, true) => "PLAYERS · INCOMING",
            _ => "PLAYERS",
        };
        draw_text(
            font,
            glyphs,
            pixels,
            heading,
            x + 15,
            cursor_y,
            15,
            MUTED,
            width - 30,
        );
        cursor_y += 16;
        let player_count = players.len().min(budget);
        let max_value = players
            .iter()
            .take(player_count)
            .map(|player| player_primary_metric(player, settings))
            .fold(0.0_f64, f64::max);
        let accent = if settings.show_rolling_dps {
            CYAN
        } else if settings.show_total_damage {
            VIOLET
        } else {
            RED
        };
        for player in players.into_iter().take(player_count) {
            if cursor_y + row_height > y + height {
                break;
            }
            draw_stat_row(
                font,
                glyphs,
                pixels,
                x + 15,
                cursor_y,
                width - 30,
                &player.player,
                &player_metric_summary(player, settings),
                ratio(player_primary_metric(player, settings), max_value),
                accent,
            );
            cursor_y += row_height;
        }
        budget = budget.saturating_sub(player_count);
        cursor_y += 4;
    }

    if settings.show_attacks && !snapshot.attacks.is_empty() && budget > 0 {
        if cursor_y + 18 <= y + height {
            draw_text(
                font,
                glyphs,
                pixels,
                "ATTACKS",
                x + 15,
                cursor_y,
                15,
                MUTED,
                width - 30,
            );
            cursor_y += 16;
        }
        let max_damage = snapshot
            .attacks
            .iter()
            .take(budget)
            .map(|attack| attack.total)
            .fold(0.0_f64, f64::max);
        for attack in snapshot.attacks.iter().take(budget) {
            if cursor_y + row_height > y + height {
                break;
            }
            draw_stat_row(
                font,
                glyphs,
                pixels,
                x + 15,
                cursor_y,
                width - 30,
                &attack.name,
                &format!(
                    "{}  |  {:.0}%",
                    compact_number(attack.total),
                    attack.share.max(0.0) * 100.0
                ),
                ratio(attack.total, max_damage),
                VIOLET,
            );
            cursor_y += row_height;
        }
    }

    if snapshot.players.is_empty() && snapshot.attacks.is_empty() {
        draw_text(
            font,
            glyphs,
            pixels,
            "Waiting for combat data...",
            x + 15,
            y + 54,
            18,
            DIM,
            width - 30,
        );
    }
}

fn player_primary_metric(
    player: &minmaxxer_core::PlayerStats,
    settings: &VrOverlaySettings,
) -> f64 {
    if settings.show_rolling_dps {
        player.damage.dps
    } else if settings.show_total_damage {
        player.damage.total
    } else if settings.show_incoming {
        player.incoming.damage_per_second
    } else {
        0.0
    }
}

fn player_metric_summary(
    player: &minmaxxer_core::PlayerStats,
    settings: &VrOverlaySettings,
) -> String {
    let mut values = Vec::with_capacity(3);
    if settings.show_rolling_dps {
        values.push(format!("{} DPS", compact_number(player.damage.dps)));
    }
    if settings.show_total_damage {
        values.push(format!("{} DMG", compact_number(player.damage.total)));
    }
    if settings.show_incoming {
        values.push(format!(
            "{} IN/s",
            compact_number(player.incoming.damage_per_second)
        ));
    }
    if values.is_empty() {
        "—".to_owned()
    } else {
        values.join(" | ")
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_stat_row(
    font: &Font,
    glyphs: &mut HashMap<GlyphKey, CachedGlyph>,
    pixels: &mut [u8],
    x: i32,
    y: i32,
    width: i32,
    label: &str,
    value: &str,
    fraction: f64,
    accent: Color,
) {
    draw_text(font, glyphs, pixels, label, x, y + 16, 17, WHITE, width);
    draw_text_right(
        font,
        glyphs,
        pixels,
        value,
        x + width,
        y + 34,
        14,
        MUTED,
        width,
    );
    fill_rounded_rect(pixels, x, y + 39, width, 3, 1, Color(48, 62, 84, 180));
    let bar_width = ((width as f64 * fraction.clamp(0.0, 1.0)).round() as i32).max(2);
    fill_rounded_rect(pixels, x, y + 39, bar_width, 3, 1, accent);
}

#[allow(clippy::too_many_arguments, dead_code)]
fn draw_recent_hits_column(
    font: &Font,
    glyphs: &mut HashMap<GlyphKey, CachedGlyph>,
    pixels: &mut [u8],
    snapshot: &EngineSnapshot,
    settings: &VrOverlaySettings,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) {
    fill_rounded_rect(pixels, x, y, width, height, 14, PANEL_ALT);
    draw_text(
        font,
        glyphs,
        pixels,
        "RECENT LOCAL HITS",
        x + 15,
        y + 25,
        15,
        MUTED,
        width - 30,
    );

    let maximum_by_height = ((height - 34).max(0) / 36) as usize;
    let requested = settings.recent_hit_rows as usize;
    let hits = recent_local_hits(snapshot, requested.min(maximum_by_height));
    if hits.is_empty() {
        draw_text(
            font,
            glyphs,
            pixels,
            "Your latest hits will appear here.",
            x + 15,
            y + 58,
            18,
            DIM,
            width - 30,
        );
        return;
    }

    let mut row_y = y + 34;
    for (index, hit) in hits.into_iter().enumerate() {
        if index % 2 == 0 {
            fill_rounded_rect(
                pixels,
                x + 8,
                row_y - 2,
                width - 16,
                33,
                7,
                Color(27, 39, 62, 128),
            );
        }
        let amount = compact_number(hit.amount);
        draw_text(
            font,
            glyphs,
            pixels,
            &amount,
            x + 17,
            row_y + 21,
            20,
            ORANGE,
            105,
        );
        let damage_type = friendly_label(&hit.damage_type);
        draw_text(
            font,
            glyphs,
            pixels,
            &damage_type,
            x + 126,
            row_y + 20,
            17,
            WHITE,
            width - 215,
        );
        draw_text_right(
            font,
            glyphs,
            pixels,
            &format_age(hit.age_seconds),
            x + width - 16,
            row_y + 20,
            15,
            MUTED,
            72,
        );
        row_y += 36;
    }
}

fn recent_local_hits(
    snapshot: &EngineSnapshot,
    limit: usize,
) -> Vec<&minmaxxer_core::aggregate::RecentHit> {
    snapshot.recent_hits.iter().take(limit).collect()
}

fn recent_incoming_events(
    snapshot: &EngineSnapshot,
    limit: usize,
) -> Vec<&minmaxxer_core::GameEvent> {
    snapshot
        .recent_events
        .iter()
        .rev()
        .filter(|event| event.kind == minmaxxer_core::EventKind::DamageTaken)
        .take(limit)
        .collect()
}

fn friendly_entity_label(value: &str) -> String {
    humanize_identifier(value)
}

fn friendly_incoming_source(value: &str) -> String {
    let source = value.trim();
    if source.is_empty() {
        return "UNKNOWN SOURCE".to_owned();
    }
    if let Some(without_open) = source.strip_prefix('(') {
        if let Some((actor, attack)) = without_open.split_once(')') {
            let actor = humanize_identifier(actor);
            let attack = humanize_identifier(attack);
            return if attack.is_empty() {
                actor
            } else {
                format!("{actor} - {attack}")
            };
        }
    }
    humanize_identifier(source)
}

fn humanize_identifier(value: &str) -> String {
    let value = value
        .trim()
        .trim_end_matches("(Clone)")
        .trim()
        .strip_prefix("Stage_")
        .or_else(|| value.trim().strip_prefix("stage_"))
        .unwrap_or_else(|| value.trim());
    let value = value
        .strip_prefix("attack_")
        .or_else(|| value.strip_prefix("Attack_"))
        .unwrap_or(value);
    let mut output = String::with_capacity(value.len() + 8);
    let mut previous: Option<char> = None;
    for character in value.chars() {
        if character == '_' || character == '-' || character.is_whitespace() {
            if !output.ends_with(' ') && !output.is_empty() {
                output.push(' ');
            }
            previous = None;
            continue;
        }
        let boundary = previous.is_some_and(|previous| {
            (previous.is_ascii_lowercase() && character.is_ascii_uppercase())
                || (previous.is_ascii_alphabetic() && character.is_ascii_digit())
        });
        if boundary && !output.ends_with(' ') {
            output.push(' ');
        }
        output.push(character);
        previous = Some(character);
    }
    output
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_uppercase()
}

fn friendly_label(value: &str) -> String {
    let mut label = value.replace(['_', '-'], " ");
    if let Some(first) = label.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    label
}

fn ratio(value: f64, maximum: f64) -> f64 {
    if value.is_finite() && maximum.is_finite() && maximum > 0.0 {
        value / maximum
    } else {
        0.0
    }
}

fn compact_number(value: f64) -> String {
    if !value.is_finite() {
        return "--".to_owned();
    }
    let absolute = value.abs();
    if absolute >= 1_000_000_000.0 {
        format!("{:.1}b", value / 1_000_000_000.0)
    } else if absolute >= 1_000_000.0 {
        format!("{:.1}m", value / 1_000_000.0)
    } else if absolute >= 1_000.0 {
        format!("{:.1}k", value / 1_000.0)
    } else if absolute >= 100.0 {
        format!("{value:.0}")
    } else if absolute >= 10.0 {
        format!("{value:.1}")
    } else {
        format!("{value:.2}")
    }
}

fn format_age(seconds: f64) -> String {
    if seconds < 1.0 {
        "<1s".to_owned()
    } else if seconds < 60.0 {
        format!("{seconds:.0}s")
    } else {
        format!("{:.0}m", seconds / 60.0)
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_text(
    font: &Font,
    glyphs: &mut HashMap<GlyphKey, CachedGlyph>,
    pixels: &mut [u8],
    text: &str,
    x: i32,
    baseline_y: i32,
    pixel_size: u16,
    color: Color,
    maximum_width: i32,
) {
    let mut pen_x = x as f32;
    let right = (x + maximum_width.max(0)) as f32;
    let size = pixel_size as f32;
    let mut previous = None;

    for character in text.chars().filter(|character| !character.is_control()) {
        if let Some(previous) = previous {
            pen_x += font
                .horizontal_kern(previous, character, size)
                .unwrap_or_default();
        }
        let key = GlyphKey {
            character,
            pixels: pixel_size,
        };
        let glyph = glyphs.entry(key).or_insert_with(|| {
            let (metrics, coverage) = font.rasterize(character, size);
            CachedGlyph { metrics, coverage }
        });
        if pen_x + glyph.metrics.advance_width > right {
            break;
        }
        let glyph_x = pen_x.round() as i32 + glyph.metrics.xmin;
        let glyph_y = baseline_y - glyph.metrics.height as i32 - glyph.metrics.ymin;
        blend_glyph(pixels, glyph_x, glyph_y, glyph, color);
        pen_x += glyph.metrics.advance_width;
        previous = Some(character);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_text_right(
    font: &Font,
    glyphs: &mut HashMap<GlyphKey, CachedGlyph>,
    pixels: &mut [u8],
    text: &str,
    right: i32,
    baseline_y: i32,
    pixel_size: u16,
    color: Color,
    maximum_width: i32,
) {
    let width = measure_text(font, text, pixel_size).ceil() as i32;
    let visible_width = width.min(maximum_width.max(0));
    draw_text(
        font,
        glyphs,
        pixels,
        text,
        right - visible_width,
        baseline_y,
        pixel_size,
        color,
        visible_width,
    );
}

fn measure_text(font: &Font, text: &str, pixel_size: u16) -> f32 {
    let size = pixel_size as f32;
    let mut width = 0.0;
    let mut previous = None;
    for character in text.chars().filter(|character| !character.is_control()) {
        if let Some(previous) = previous {
            width += font
                .horizontal_kern(previous, character, size)
                .unwrap_or_default();
        }
        width += font.metrics(character, size).advance_width;
        previous = Some(character);
    }
    width
}

fn blend_glyph(pixels: &mut [u8], x: i32, y: i32, glyph: &CachedGlyph, color: Color) {
    for glyph_y in 0..glyph.metrics.height {
        let destination_y = y + glyph_y as i32;
        if !(0..HUD_TEXTURE_HEIGHT as i32).contains(&destination_y) {
            continue;
        }
        for glyph_x in 0..glyph.metrics.width {
            let destination_x = x + glyph_x as i32;
            if !(0..HUD_TEXTURE_WIDTH as i32).contains(&destination_x) {
                continue;
            }
            let coverage = glyph.coverage[glyph_y * glyph.metrics.width + glyph_x];
            if coverage == 0 {
                continue;
            }
            let alpha = ((coverage as u16 * color.3 as u16 + 127) / 255) as u8;
            blend_pixel(
                pixels,
                destination_x,
                destination_y,
                Color(color.0, color.1, color.2, alpha),
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_line(
    pixels: &mut [u8],
    mut x0: i32,
    mut y0: i32,
    x1: i32,
    y1: i32,
    color: Color,
    thickness: i32,
) {
    let delta_x = (x1 - x0).abs();
    let step_x = if x0 < x1 { 1 } else { -1 };
    let delta_y = -(y1 - y0).abs();
    let step_y = if y0 < y1 { 1 } else { -1 };
    let mut error = delta_x + delta_y;
    let radius = (thickness.max(1) - 1) / 2;
    loop {
        for offset_y in -radius..=radius {
            for offset_x in -radius..=radius {
                blend_pixel(pixels, x0 + offset_x, y0 + offset_y, color);
            }
        }
        if x0 == x1 && y0 == y1 {
            break;
        }
        let doubled = error * 2;
        if doubled >= delta_y {
            error += delta_y;
            x0 += step_x;
        }
        if doubled <= delta_x {
            error += delta_x;
            y0 += step_y;
        }
    }
}

fn fill_circle(pixels: &mut [u8], center_x: i32, center_y: i32, radius: i32, color: Color) {
    for y in (center_y - radius)..=(center_y + radius) {
        for x in (center_x - radius)..=(center_x + radius) {
            let dx = x - center_x;
            let dy = y - center_y;
            if dx * dx + dy * dy <= radius * radius {
                blend_pixel(pixels, x, y, color);
            }
        }
    }
}

fn fill_rounded_rect(
    pixels: &mut [u8],
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    radius: i32,
    color: Color,
) {
    if width <= 0 || height <= 0 || color.3 == TRANSPARENT.3 {
        return;
    }
    // `(dimension - 1) / 2` keeps the integer corner-centre clamp ordered for even dimensions.
    let radius = radius.max(0).min((width - 1) / 2).min((height - 1) / 2);
    let left_center = x + radius;
    let right_center = x + width - radius - 1;
    let top_center = y + radius;
    let bottom_center = y + height - radius - 1;
    let radius_squared = radius * radius;

    for destination_y in y.max(0)..(y + height).min(HUD_TEXTURE_HEIGHT as i32) {
        for destination_x in x.max(0)..(x + width).min(HUD_TEXTURE_WIDTH as i32) {
            let nearest_x = destination_x.clamp(left_center, right_center);
            let nearest_y = destination_y.clamp(top_center, bottom_center);
            let dx = destination_x - nearest_x;
            let dy = destination_y - nearest_y;
            if radius == 0 || dx * dx + dy * dy <= radius_squared {
                blend_pixel(pixels, destination_x, destination_y, color);
            }
        }
    }
}

fn blend_pixel(pixels: &mut [u8], x: i32, y: i32, source: Color) {
    if !(0..HUD_TEXTURE_WIDTH as i32).contains(&x)
        || !(0..HUD_TEXTURE_HEIGHT as i32).contains(&y)
        || source.3 == 0
    {
        return;
    }
    let index = (y as usize * HUD_TEXTURE_WIDTH + x as usize) * HUD_BYTES_PER_PIXEL;
    let destination_alpha = pixels[index + 3] as u64;
    if destination_alpha == 0 || source.3 == 255 {
        pixels[index] = source.0;
        pixels[index + 1] = source.1;
        pixels[index + 2] = source.2;
        pixels[index + 3] = source.3;
        return;
    }
    let source_alpha = source.3 as u64;
    let inverse_source = 255 - source_alpha;
    let output_alpha_numerator = source_alpha * 255 + destination_alpha * inverse_source;
    if output_alpha_numerator == 0 {
        return;
    }

    for (channel, source_channel) in [source.0, source.1, source.2].into_iter().enumerate() {
        let destination_channel = pixels[index + channel] as u64;
        let numerator = source_channel as u64 * source_alpha * 255
            + destination_channel * destination_alpha * inverse_source;
        pixels[index + channel] =
            ((numerator + output_alpha_numerator / 2) / output_alpha_numerator) as u8;
    }
    pixels[index + 3] = ((output_alpha_numerator + 127) / 255) as u8;
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use minmaxxer_core::{
        DamageTotals, EventKind, GameEvent, IncomingTotals, PlayerStats, TimelinePoint,
    };

    #[test]
    fn legacy_vr_settings_inherit_run_context_without_enabling_loadout() {
        let settings: VrOverlaySettings = serde_json::from_value(serde_json::json!({})).unwrap();

        assert!(settings.show_phase);
        assert!(settings.show_boss_number);
        assert!(!settings.show_loadout);
    }

    #[test]
    fn settings_sanitize_openvr_ranges_and_non_finite_values() {
        let settings = VrOverlaySettings {
            width_m: f32::NAN,
            x: f32::INFINITY,
            y: -50.0,
            z: 4.0,
            opacity: 8.0,
            curvature: -3.0,
            rows: 0,
            recent_hit_rows: u8::MAX,
            ..VrOverlaySettings::default()
        }
        .sanitized();

        assert_eq!(settings.width_m, 0.86);
        assert_eq!(settings.x, 0.30);
        assert_eq!(settings.y, -5.0);
        assert_eq!(settings.z, -0.05);
        assert_eq!(settings.opacity, 1.0);
        assert_eq!(settings.curvature, 0.0);
        assert_eq!(settings.rows, 1);
        assert_eq!(settings.recent_hit_rows, 8);
    }

    #[test]
    fn player_rows_respect_each_native_metric_toggle() {
        let player = PlayerStats {
            player: "Local".to_owned(),
            damage: DamageTotals {
                total: 12_000.0,
                dps: 1_200.0,
                ..DamageTotals::default()
            },
            incoming: IncomingTotals {
                total: 900.0,
                damage_per_second: 90.0,
                ..IncomingTotals::default()
            },
            ..PlayerStats::default()
        };
        let mut settings = VrOverlaySettings {
            show_rolling_dps: true,
            show_total_damage: false,
            show_incoming: false,
            ..VrOverlaySettings::default()
        };

        let dps = player_metric_summary(&player, &settings);
        assert!(dps.contains("DPS"));
        assert!(!dps.contains("DMG"));
        assert!(!dps.contains("IN/s"));
        assert_eq!(player_primary_metric(&player, &settings), 1_200.0);

        settings.show_rolling_dps = false;
        settings.show_total_damage = true;
        let damage = player_metric_summary(&player, &settings);
        assert!(!damage.contains("DPS"));
        assert!(damage.contains("DMG"));
        assert!(!damage.contains("IN/s"));
        assert_eq!(player_primary_metric(&player, &settings), 12_000.0);

        settings.show_total_damage = false;
        settings.show_incoming = true;
        let incoming = player_metric_summary(&player, &settings);
        assert!(!incoming.contains("DPS"));
        assert!(!incoming.contains("DMG"));
        assert!(incoming.contains("IN/s"));
        assert_eq!(player_primary_metric(&player, &settings), 90.0);

        settings.show_rolling_dps = true;
        settings.show_total_damage = true;
        let combined = player_metric_summary(&player, &settings);
        assert!(combined.contains("DPS"));
        assert!(combined.contains("DMG"));
        assert!(combined.contains("IN/s"));
        assert!(
            combined.len() < 40,
            "default VR summary should stay compact"
        );
    }

    #[test]
    fn raw_openvr_strings_are_nul_terminated() {
        assert_eq!(OVERLAY_KEY.as_bytes().last(), Some(&0));
        assert_eq!(OVERLAY_NAME.as_bytes().last(), Some(&0));
        assert_eq!(
            OVERLAY_KEY
                .as_bytes()
                .iter()
                .filter(|byte| **byte == 0)
                .count(),
            1
        );
        assert_eq!(
            OVERLAY_NAME
                .as_bytes()
                .iter()
                .filter(|byte| **byte == 0)
                .count(),
            1
        );
    }

    #[test]
    fn renderer_keeps_straight_alpha_on_transparent_pixels() {
        let mut pixels = vec![0; HUD_TEXTURE_WIDTH * HUD_TEXTURE_HEIGHT * 4];
        blend_pixel(&mut pixels, 5, 7, Color(200, 100, 50, 128));
        let index = (7 * HUD_TEXTURE_WIDTH + 5) * 4;
        assert_eq!(&pixels[index..index + 4], &[200, 100, 50, 128]);
    }

    #[test]
    fn right_grip_press_immediately_moves_and_release_finishes() {
        let mut gesture = GrabGesture::default();
        assert_eq!(
            gesture.update(right_grip(true, true), Duration::from_millis(50)),
            GestureAction::BeginMoving
        );
        assert!(gesture.is_moving());
        assert_eq!(gesture.placement_state(true), VrPlacementState::Moving);
        assert_eq!(
            gesture.update(right_grip(false, true), Duration::from_millis(16)),
            GestureAction::FinishMoving
        );
        assert!(!gesture.is_moving());
        assert_eq!(gesture.placement_state(true), VrPlacementState::Listening);
    }

    #[test]
    fn release_keeps_move_mode_listening_for_a_second_grab() {
        let mut gesture = GrabGesture::default();
        assert_eq!(
            gesture.update(right_grip(true, true), Duration::from_millis(50)),
            GestureAction::BeginMoving
        );
        assert_eq!(
            gesture.update(right_grip(false, true), Duration::from_millis(16)),
            GestureAction::FinishMoving
        );
        assert_eq!(
            gesture.update(right_grip(true, true), Duration::from_millis(50)),
            GestureAction::BeginMoving
        );
        assert!(gesture.is_moving());
    }

    #[test]
    fn release_waits_for_a_valid_pose_before_freezing() {
        let mut gesture = GrabGesture::default();
        assert_eq!(
            gesture.update(right_grip(true, true), Duration::from_millis(50)),
            GestureAction::BeginMoving
        );
        assert_eq!(
            gesture.update(right_grip(false, false), Duration::from_millis(16)),
            GestureAction::None
        );
        assert!(gesture.is_moving());
        assert_eq!(
            gesture.update(right_grip(false, true), Duration::from_millis(16)),
            GestureAction::FinishMoving
        );
        assert_eq!(gesture.placement_state(true), VrPlacementState::Listening);
    }

    #[test]
    fn compositor_state_cache_only_reapplies_changed_properties_and_transform_kinds() {
        let mut applied = AppliedOverlayState::default();
        assert!(applied.needs_width(0.8));
        assert!(applied.needs_opacity(0.9));
        assert!(applied.needs_curvature(0.1));
        applied.width_m = Some(0.8);
        applied.opacity = Some(0.9);
        applied.curvature = Some(0.1);
        assert!(!applied.needs_width(0.8));
        assert!(!applied.needs_opacity(0.9));
        assert!(!applied.needs_curvature(0.1));

        let transform = configured_transform(&VrOverlaySettings::default());
        let hmd = AppliedTransform {
            kind: TransformKind::Hmd,
            transform,
        };
        assert_eq!(applied.transform_change(hmd), (true, false));
        applied.transform = Some(hmd);
        assert_eq!(applied.transform_change(hmd), (false, false));
        let controller = AppliedTransform {
            kind: TransformKind::Controller(2),
            transform,
        };
        assert_eq!(applied.transform_change(controller), (true, true));
    }

    #[test]
    fn controller_relative_matrix_round_trip_preserves_hmd_pose() {
        let world_from_hmd = RigidTransform::from_rows([
            [0.0, 0.0, 1.0, 1.25],
            [0.0, 1.0, 0.0, 1.70],
            [-1.0, 0.0, 0.0, -0.40],
        ]);
        let world_from_controller = RigidTransform::from_rows([
            [1.0, 0.0, 0.0, 0.45],
            [0.0, 0.0, -1.0, 1.10],
            [0.0, 1.0, 0.0, -0.85],
        ]);
        let hmd_to_overlay = RigidTransform::from_rows([
            [1.0, 0.0, 0.0, 0.30],
            [0.0, 1.0, 0.0, 0.08],
            [0.0, 0.0, 1.0, -1.05],
        ]);

        let world_from_overlay = world_from_hmd.multiply(hmd_to_overlay);
        let controller_to_overlay = world_from_controller
            .inverse_rigid()
            .multiply(world_from_overlay);
        let recovered = world_from_hmd
            .inverse_rigid()
            .multiply(world_from_controller.multiply(controller_to_overlay));
        assert_transform_close(recovered, hmd_to_overlay);
        assert_transform_close(
            world_from_hmd.inverse_rigid().multiply(world_from_hmd),
            RigidTransform::from_rows([
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
            ]),
        );
    }

    #[test]
    fn runtime_controller_pose_wins_during_snapshot_redraws() {
        let runtime = RigidTransform::from_rows([
            [0.0, 0.0, 1.0, -0.22],
            [0.0, 1.0, 0.0, 0.31],
            [-1.0, 0.0, 0.0, -0.74],
        ]);
        let settings = VrOverlaySettings {
            x: 4.0,
            y: -2.0,
            z: -8.0,
            ..VrOverlaySettings::default()
        };
        assert_eq!(effective_transform(&settings, Some(runtime)), runtime);
        assert_eq!(
            effective_transform(&settings, None),
            configured_transform(&settings)
        );
    }

    #[test]
    fn public_worker_spawns_without_an_entered_tokio_reactor() {
        let (_snapshot_tx, snapshot_rx) = watch::channel(EngineSnapshot::default());
        let (_settings_tx, settings_rx) = watch::channel(VrOverlaySettings::default());

        let mut worker = spawn_vr_overlay(snapshot_rx, settings_rx);

        assert!(!worker.status.borrow().enabled);
        worker.request_shutdown();
    }

    #[tokio::test]
    async fn disabled_worker_shuts_down_without_openvr_initialization() {
        let (_snapshot_tx, snapshot_rx) = watch::channel(EngineSnapshot::default());
        let (_settings_tx, settings_rx) = watch::channel(VrOverlaySettings::default());
        let (status_tx, status_rx) = watch::channel(VrOverlayStatus::default());
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let worker = tokio::spawn(run_vr_overlay(
            snapshot_rx,
            settings_rx,
            status_tx,
            shutdown_rx,
        ));

        shutdown_tx.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(1), worker)
            .await
            .expect("disabled worker should not poll or block")
            .expect("worker should exit cleanly");
        assert_eq!(*status_rx.borrow(), VrOverlayStatus::default());
    }

    #[test]
    fn recent_hits_use_the_clock_aged_canonical_feed() {
        let time = NaiveDate::from_ymd_opt(2026, 7, 21)
            .unwrap()
            .and_hms_opt(20, 0, 0)
            .unwrap();
        let snapshot = EngineSnapshot {
            recent_hits: vec![
                minmaxxer_core::aggregate::RecentHit {
                    timestamp: time + chrono::Duration::seconds(7),
                    amount: 240.0,
                    damage_type: "strike".to_owned(),
                    age_seconds: 13.0,
                },
                minmaxxer_core::aggregate::RecentHit {
                    timestamp: time + chrono::Duration::seconds(1),
                    amount: 35.0,
                    damage_type: "non-strike".to_owned(),
                    age_seconds: 19.0,
                },
            ],
            ..EngineSnapshot::default()
        };

        let hits = recent_local_hits(&snapshot, 5);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].amount, 240.0);
        assert_eq!(hits[0].age_seconds, 13.0);
        assert_eq!(hits[1].damage_type, "non-strike");
    }

    #[test]
    fn dense_context_names_phase_round_stage_boss_and_focus() {
        let time = NaiveDate::from_ymd_opt(2026, 7, 22)
            .unwrap()
            .and_hms_opt(23, 14, 40)
            .unwrap();
        let snapshot = EngineSnapshot {
            stage: Some("Stage_LostElysia".to_owned()),
            encounter: minmaxxer_core::aggregate::LiveEncounter {
                name: "CorusPhase2".to_owned(),
                kind: "boss".to_owned(),
                active: true,
                ..minmaxxer_core::aggregate::LiveEncounter::default()
            },
            run_context: minmaxxer_core::RunContext {
                progress: Some(0.604),
                phase_name: Some("UMBRA".to_owned()),
                boss_number: Some(9),
                boss_number_inferred: false,
                boss_subphase: Some(2),
            },
            focus: Some(minmaxxer_core::aggregate::FocusSignal {
                player: "Local Player".to_owned(),
                entity: "CorusPhase2".to_owned(),
                observed_at: time,
                age_seconds: 2.0,
                confidence: "inferred".to_owned(),
                evidence: "boss_network_ownership".to_owned(),
                corroborating_hits: 0,
                corroborated_at: None,
                source_note: "Not authoritative hate.".to_owned(),
            }),
            ..EngineSnapshot::default()
        };

        let context = native_hud_context(&snapshot, &VrOverlaySettings::default());
        assert!(context.line.contains("PHASE UMBRA 60%"));
        assert!(context.line.contains("ROUND 09/13"));
        assert!(context.line.contains("STAGE LOST ELYSIA"));
        assert!(context.line.contains("BOSS CORUS PHASE 2"));
        assert!(context.focus.contains("LOCAL PLAYER"));
        assert!(context.focus.contains("INFERRED CONFIDENCE"));
    }

    #[test]
    fn incoming_feed_uses_newest_damage_taken_and_prettifies_raw_sources() {
        let time = NaiveDate::from_ymd_opt(2026, 7, 22)
            .unwrap()
            .and_hms_opt(23, 54, 0)
            .unwrap();
        let mut outgoing = event(1, time, EventKind::DamageDealt, "Local");
        outgoing.source = Some("Local".to_owned());
        let mut older = event(
            2,
            time + chrono::Duration::seconds(1),
            EventKind::DamageTaken,
            "Local",
        );
        older.source = Some("(Golden Grouch) attack_judgementAOE".to_owned());
        let mut newer = event(
            3,
            time + chrono::Duration::seconds(2),
            EventKind::DamageTaken,
            "Local",
        );
        newer.source = Some("machinegunShooter2".to_owned());
        let snapshot = EngineSnapshot {
            recent_events: vec![outgoing, older, newer],
            ..EngineSnapshot::default()
        };

        let incoming = recent_incoming_events(&snapshot, 5);
        assert_eq!(incoming.len(), 2);
        assert_eq!(incoming[0].sequence, 3);
        assert_eq!(incoming[1].sequence, 2);
        assert_eq!(
            friendly_incoming_source("(Golden Grouch) attack_judgementAOE"),
            "GOLDEN GROUCH - JUDGEMENT AOE"
        );
        assert_eq!(
            friendly_incoming_source("machinegunShooter2"),
            "MACHINEGUN SHOOTER 2"
        );
        assert_eq!(friendly_incoming_source(""), "UNKNOWN SOURCE");
    }

    #[test]
    fn unavailable_native_fields_are_never_presented_as_zeroes() {
        assert_eq!(
            UNAVAILABLE_TELEMETRY,
            [
                ("HP", "NOT LOGGED"),
                ("HEALING", "NOT LOGGED"),
                ("GEMS", "NOT LOGGED"),
                ("SONG", "NOT LOGGED"),
            ]
        );
        assert!(UNAVAILABLE_TELEMETRY
            .iter()
            .all(|(_, value)| *value == "NOT LOGGED"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn renderer_produces_reusable_rgba_frame_with_transparent_margin() {
        let mut renderer = HudRenderer::new().expect("Windows includes Segoe UI");
        let settings = VrOverlaySettings::default();
        let first_pointer = renderer.pixels().as_ptr();
        assert_eq!(renderer.width(), HUD_TEXTURE_WIDTH);
        assert_eq!(renderer.height(), HUD_TEXTURE_HEIGHT);
        let frame = renderer.render(&EngineSnapshot::default(), &settings);

        assert_eq!(frame.len(), HUD_TEXTURE_WIDTH * HUD_TEXTURE_HEIGHT * 4);
        assert_eq!(&frame[0..4], &[0, 0, 0, 0]);
        let panel_pixel = (256 * HUD_TEXTURE_WIDTH + 512) * 4;
        assert!(frame[panel_pixel + 3] > 0);
        assert_eq!(
            first_pointer,
            frame.as_ptr(),
            "frame buffer should be reused"
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn recent_hit_setting_changes_the_cpu_render() {
        let time = NaiveDate::from_ymd_opt(2026, 7, 21)
            .unwrap()
            .and_hms_opt(20, 0, 0)
            .unwrap();
        let mut snapshot = EngineSnapshot {
            observed_player: Some("Local".to_owned()),
            last_event_at: Some(time + chrono::Duration::seconds(2)),
            ..EngineSnapshot::default()
        };
        snapshot.recent_events = vec![event(
            1,
            time + chrono::Duration::seconds(1),
            EventKind::DamageDealt,
            "Local",
        )];

        let mut renderer = HudRenderer::new().expect("Windows includes Segoe UI");
        let mut settings = VrOverlaySettings {
            show_players: false,
            show_attacks: false,
            show_recent_hits: false,
            ..VrOverlaySettings::default()
        };
        let without_hits = renderer.render(&snapshot, &settings).to_vec();
        settings.show_recent_hits = true;
        let with_hits = renderer.render(&snapshot, &settings);
        assert_ne!(without_hits, with_hits);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn current_segment_timeline_draws_a_visible_dps_graph() {
        let time = NaiveDate::from_ymd_opt(2026, 7, 22)
            .unwrap()
            .and_hms_opt(20, 0, 0)
            .unwrap();
        let snapshot = EngineSnapshot {
            encounter: minmaxxer_core::aggregate::LiveEncounter {
                name: "Corus".to_owned(),
                kind: "boss".to_owned(),
                active: true,
                ..minmaxxer_core::aggregate::LiveEncounter::default()
            },
            outgoing: DamageTotals {
                total: 12_400.0,
                rolling_5s: 900.0,
                ..DamageTotals::default()
            },
            timeline: vec![
                TimelinePoint {
                    timestamp: time,
                    rolling_dps: 220.0,
                    ..TimelinePoint::default()
                },
                TimelinePoint {
                    timestamp: time + chrono::Duration::seconds(1),
                    rolling_dps: 900.0,
                    ..TimelinePoint::default()
                },
                TimelinePoint {
                    timestamp: time + chrono::Duration::seconds(2),
                    rolling_dps: 460.0,
                    ..TimelinePoint::default()
                },
            ],
            ..EngineSnapshot::default()
        };
        let settings = VrOverlaySettings {
            show_status: false,
            show_encounter: false,
            show_phase: false,
            show_boss_number: false,
            show_focus: false,
            show_total_damage: false,
            show_incoming: false,
            show_recent_hits: false,
            ..VrOverlaySettings::default()
        };
        let mut renderer = HudRenderer::new().expect("Windows includes Segoe UI");
        let frame = renderer.render(&snapshot, &settings);

        let cyan_graph_pixels = (200..354)
            .flat_map(|y| (39..319).map(move |x| (y * HUD_TEXTURE_WIDTH + x) * 4))
            .filter(|index| frame[*index..*index + 4] == [CYAN.0, CYAN.1, CYAN.2, CYAN.3])
            .count();
        assert!(
            cyan_graph_pixels > 100,
            "the current-segment timeline should produce an obvious cyan line"
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn run_phase_and_boss_controls_change_the_native_hud() {
        let snapshot = EngineSnapshot {
            run_context: minmaxxer_core::RunContext {
                progress: Some(0.84),
                phase_name: Some("ECLIPSE".to_owned()),
                boss_number: Some(11),
                boss_number_inferred: true,
                boss_subphase: Some(2),
            },
            ..EngineSnapshot::default()
        };
        let mut settings = VrOverlaySettings {
            show_status: false,
            show_encounter: false,
            show_phase: false,
            show_boss_number: false,
            show_focus: false,
            show_rolling_dps: false,
            show_total_damage: false,
            show_incoming: false,
            show_players: false,
            show_attacks: false,
            show_recent_hits: false,
            show_loadout: false,
            ..VrOverlaySettings::default()
        };
        let mut renderer = HudRenderer::new().expect("Windows includes Segoe UI");

        let hidden = renderer.render(&snapshot, &settings).to_vec();
        settings.show_phase = true;
        let phase = renderer.render(&snapshot, &settings).to_vec();
        assert!(changed_pixels_in_context_row(&hidden, &phase) > 0);

        settings.show_boss_number = true;
        let boss = renderer.render(&snapshot, &settings).to_vec();
        assert!(changed_pixels_in_context_row(&phase, &boss) > 0);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn boss_target_placeholders_are_rendered_by_default() {
        let mut renderer = HudRenderer::new().expect("Windows includes Segoe UI");
        let mut settings = VrOverlaySettings {
            show_status: false,
            show_encounter: false,
            show_focus: false,
            ..VrOverlaySettings::default()
        };
        let mut snapshot = EngineSnapshot::default();

        let hidden = renderer.render(&snapshot, &settings).to_vec();
        settings.show_focus = true;
        let no_boss = renderer.render(&snapshot, &settings).to_vec();
        assert!(changed_pixels_in_focus_row(&hidden, &no_boss) > 0);

        snapshot.encounter = minmaxxer_core::aggregate::LiveEncounter {
            name: "Astral Sovereign".to_owned(),
            kind: "boss".to_owned(),
            active: true,
            ..minmaxxer_core::aggregate::LiveEncounter::default()
        };
        let acquiring = renderer.render(&snapshot, &settings).to_vec();
        assert!(changed_pixels_in_focus_row(&no_boss, &acquiring) > 0);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn inferred_boss_target_signal_changes_the_native_hud_pixels() {
        let time = NaiveDate::from_ymd_opt(2026, 7, 21)
            .unwrap()
            .and_hms_opt(20, 0, 0)
            .unwrap();
        let settings = VrOverlaySettings {
            show_status: false,
            show_encounter: false,
            ..VrOverlaySettings::default()
        };
        let mut snapshot = EngineSnapshot {
            encounter: minmaxxer_core::aggregate::LiveEncounter {
                name: "Astral Sovereign".to_owned(),
                kind: "boss".to_owned(),
                active: true,
                ..minmaxxer_core::aggregate::LiveEncounter::default()
            },
            ..EngineSnapshot::default()
        };
        let mut renderer = HudRenderer::new().expect("Windows includes Segoe UI");
        let acquiring = renderer.render(&snapshot, &settings).to_vec();

        snapshot.focus = Some(minmaxxer_core::aggregate::FocusSignal {
            player: "Mirai".to_owned(),
            entity: "Astral Sovereign".to_owned(),
            observed_at: time,
            age_seconds: 2.4,
            confidence: "inferred".to_owned(),
            evidence: "boss_network_ownership".to_owned(),
            corroborating_hits: 0,
            corroborated_at: None,
            source_note: "Network-ownership signal; not authoritative hate.".to_owned(),
        });
        let signal = renderer.render(&snapshot, &settings).to_vec();

        assert!(changed_pixels_in_focus_row(&acquiring, &signal) > 0);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn controller_placement_state_adds_native_hud_feedback() {
        let mut renderer = HudRenderer::new().expect("Windows includes Segoe UI");
        let snapshot = EngineSnapshot::default();
        let settings = VrOverlaySettings::default();
        let regular = renderer.render(&snapshot, &settings).to_vec();
        let moving = renderer.render_with_placement(&snapshot, &settings, VrPlacementState::Moving);
        assert_ne!(regular, moving);
    }

    fn event(
        sequence: u64,
        timestamp: chrono::NaiveDateTime,
        kind: EventKind,
        player: &str,
    ) -> GameEvent {
        GameEvent {
            sequence,
            timestamp,
            kind,
            player: Some(player.to_owned()),
            session_id: None,
            world: Some("Ecliptica".to_owned()),
            instance: None,
            stage: None,
            class_name: None,
            boss: None,
            phase: None,
            amount: Some(123.0),
            damage_type: Some("strike".to_owned()),
            source: None,
            target: None,
            entity: None,
            pool_id: None,
            enemy_id: None,
            user_id: None,
            message: None,
            raw: String::new(),
        }
    }

    fn right_grip(right: bool, right_pose_available: bool) -> GripInput {
        GripInput {
            right_grip: Some(right),
            controllers_available: true,
            right_pose_available,
        }
    }

    #[cfg(target_os = "windows")]
    fn changed_pixels_in_focus_row(left: &[u8], right: &[u8]) -> usize {
        (103..141)
            .flat_map(|y| (24..1000).map(move |x| (y * HUD_TEXTURE_WIDTH + x) * 4))
            .filter(|index| left[*index..*index + 4] != right[*index..*index + 4])
            .count()
    }

    #[cfg(target_os = "windows")]
    fn changed_pixels_in_context_row(left: &[u8], right: &[u8]) -> usize {
        (72..104)
            .flat_map(|y| (24..1000).map(move |x| (y * HUD_TEXTURE_WIDTH + x) * 4))
            .filter(|index| left[*index..*index + 4] != right[*index..*index + 4])
            .count()
    }

    fn assert_transform_close(left: RigidTransform, right: RigidTransform) {
        for row in 0..3 {
            for column in 0..4 {
                assert!(
                    (left.0[row][column] - right.0[row][column]).abs() < 1.0e-5,
                    "matrix differs at [{row}][{column}]: {} != {}",
                    left.0[row][column],
                    right.0[row][column]
                );
            }
        }
    }
}
