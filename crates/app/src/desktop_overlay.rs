use crate::config::{AppConfig, DesktopOverlaySettings, OverlayProfile};
use anyhow::{Context, Result};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use tauri::{
    webview::Color, AppHandle, Manager, PhysicalPosition, PhysicalSize, Theme, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder,
};
use tokio::sync::watch;
use url::Url;

const WINDOW_LABEL: &str = "desktop-overlay";

#[derive(Debug, Clone)]
pub struct DesktopOverlayRuntimeConfig {
    pub settings: DesktopOverlaySettings,
    pub profile: OverlayProfile,
}

impl DesktopOverlayRuntimeConfig {
    pub fn from_app_config(config: &AppConfig) -> Self {
        let profile = config
            .overlay_profiles
            .iter()
            .find(|profile| profile.id == config.desktop_overlay.profile)
            .or_else(|| config.overlay_profiles.first())
            .cloned()
            .unwrap_or_default();
        Self {
            settings: config.desktop_overlay.clone(),
            profile,
        }
    }
}

pub struct DesktopOverlayHandle {
    stop: mpsc::Sender<()>,
}

impl DesktopOverlayHandle {
    pub fn stop(&self) {
        let _ = self.stop.send(());
    }
}

pub fn spawn_desktop_overlay(
    app: AppHandle,
    origin: Url,
    settings: watch::Receiver<DesktopOverlayRuntimeConfig>,
) -> Result<DesktopOverlayHandle> {
    let (stop, stop_rx) = mpsc::channel();
    thread::Builder::new()
        .name("minmaxxer-desktop-overlay".to_owned())
        .spawn(move || desktop_overlay_loop(app, origin, settings, stop_rx))
        .context("failed spawning desktop overlay manager")?;
    Ok(DesktopOverlayHandle { stop })
}

fn desktop_overlay_loop(
    app: AppHandle,
    origin: Url,
    settings_rx: watch::Receiver<DesktopOverlayRuntimeConfig>,
    stop: mpsc::Receiver<()>,
) {
    let mut last_click_through = None;
    let mut last_url: Option<Url> = None;
    loop {
        if stop.recv_timeout(Duration::from_millis(250)).is_ok() {
            if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
                let _ = window.close();
            }
            return;
        }
        let runtime = settings_rx.borrow().clone();
        let settings = &runtime.settings;
        if !settings.enabled {
            if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
                let _ = window.destroy();
            }
            last_click_through = None;
            last_url = None;
            continue;
        }

        let desired_url = overlay_url(&origin, &runtime);

        if app.get_webview_window(WINDOW_LABEL).is_none() {
            let app_for_create = app.clone();
            let url = desired_url.clone();
            let width = settings.width;
            let height = settings.height;
            let click_through = settings.click_through;
            let _ = app.run_on_main_thread(move || {
                if app_for_create.get_webview_window(WINDOW_LABEL).is_some() {
                    return;
                }
                match create_window(&app_for_create, url, width, height) {
                    Ok(window) => {
                        let _ = window.set_ignore_cursor_events(click_through);
                    }
                    Err(error) => tracing::error!(%error, "failed creating desktop HUD window"),
                }
            });
            last_url = Some(desired_url);
            last_click_through = Some(click_through);
            continue;
        }

        let Some(window) = app.get_webview_window(WINDOW_LABEL) else {
            continue;
        };
        if last_url.as_ref() != Some(&desired_url) {
            if let Err(error) = window.navigate(desired_url.clone()) {
                tracing::warn!(%error, "could not apply updated desktop HUD profile");
            } else {
                last_url = Some(desired_url);
            }
        }
        if last_click_through != Some(settings.click_through) {
            if let Err(error) = window.set_ignore_cursor_events(settings.click_through) {
                tracing::warn!(%error, "could not update desktop HUD click-through mode");
            }
            last_click_through = Some(settings.click_through);
        }

        #[cfg(windows)]
        update_window_placement(&window, settings);
        #[cfg(not(windows))]
        {
            let _ = window.hide();
        }
    }
}

fn overlay_url(origin: &Url, runtime: &DesktopOverlayRuntimeConfig) -> Url {
    let profile = &runtime.profile;
    let settings = &runtime.settings;
    let renderer_profile = match profile.id.as_str() {
        "broadcast" | "minimal" | "vr" => profile.id.as_str(),
        _ => "broadcast",
    };
    let layout = match profile.layout.as_str() {
        "leaderboard" | "compact" | "ticker" | "hits" => profile.layout.as_str(),
        _ => "leaderboard",
    };
    let theme = match profile.theme.as_str() {
        "void" | "glass" => profile.theme.as_str(),
        _ => "void",
    };
    let accent = match profile.accent.as_str() {
        "cyan" | "mint" | "violet" | "amber" | "rose" => profile.accent.as_str(),
        "#8ff0cf" => "mint",
        _ => "cyan",
    };
    let mut show = Vec::new();
    if profile.show_dps {
        show.push("dps");
    }
    if profile.show_damage {
        show.push("damage");
    }
    if profile.show_incoming {
        show.push("incoming");
    }
    if profile.show_encounter {
        show.push("encounter");
    }
    if profile.show_hits || profile.show_recent_hits {
        show.push("hits");
    }
    if profile.show_focus {
        show.push("focus");
    }

    let mut url = origin.clone();
    url.set_path("/overlay");
    url.set_query(None);
    url.query_pairs_mut()
        .append_pair("surface", "desktop")
        .append_pair("profile", renderer_profile)
        .append_pair("layout", layout)
        .append_pair("theme", theme)
        .append_pair("accent", accent)
        .append_pair("rows", &profile.rows.clamp(1, 8).to_string())
        .append_pair("hit_rows", &profile.recent_hit_rows.clamp(1, 8).to_string())
        .append_pair("show", &show.join(","))
        .append_pair(
            "bg",
            &format!("{:.0}", settings.opacity.clamp(0.0, 1.0) * 100.0),
        )
        .append_pair(
            "scale",
            &format!("{:.0}", profile.scale.clamp(0.7, 1.6) * 100.0),
        );
    url
}

fn create_window(app: &AppHandle, url: Url, width: f64, height: f64) -> Result<WebviewWindow> {
    WebviewWindowBuilder::new(app, WINDOW_LABEL, WebviewUrl::External(url))
        .title("MINMAXXER Desktop HUD")
        .inner_size(width, height)
        .min_inner_size(260.0, 120.0)
        .theme(Some(Theme::Dark))
        .background_color(Color(0, 0, 0, 0))
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .shadow(false)
        .resizable(false)
        .focusable(false)
        .visible(false)
        .build()
        .context("Tauri could not create desktop overlay")
}

#[cfg(windows)]
fn update_window_placement(window: &WebviewWindow, settings: &DesktopOverlaySettings) {
    use windows::core::w;
    use windows::Win32::Foundation::{HWND, POINT, RECT};
    use windows::Win32::Graphics::Gdi::ClientToScreen;
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowW, GetClientRect, GetForegroundWindow, IsIconic, IsWindowVisible,
    };

    let vrchat: HWND = match unsafe { FindWindowW(w!("UnityWndClass"), w!("VRChat")) } {
        Ok(window) => window,
        Err(_) => {
            let _ = window.hide();
            return;
        }
    };
    if !unsafe { IsWindowVisible(vrchat).as_bool() } || unsafe { IsIconic(vrchat).as_bool() } {
        let _ = window.hide();
        return;
    }
    if settings.only_when_vrchat_foreground && unsafe { GetForegroundWindow() } != vrchat {
        let _ = window.hide();
        return;
    }
    if !settings.show_when_vr_active && openvr::is_hmd_present() {
        let _ = window.hide();
        return;
    }

    let mut rect = RECT::default();
    if unsafe { GetClientRect(vrchat, &mut rect) }.is_err() {
        let _ = window.hide();
        return;
    }
    let mut origin = POINT { x: 0, y: 0 };
    if !unsafe { ClientToScreen(vrchat, &mut origin) }.as_bool() {
        let _ = window.hide();
        return;
    }

    let client_width = (rect.right - rect.left).max(1);
    let client_height = (rect.bottom - rect.top).max(1);
    let width = settings.width.round().max(260.0) as i32;
    let height = settings.height.round().max(120.0) as i32;
    let right = settings.corner.contains("right");
    let bottom = settings.corner.contains("bottom");
    let x = if right {
        origin.x + client_width - width - settings.offset_x
    } else {
        origin.x + settings.offset_x
    };
    let y = if bottom {
        origin.y + client_height - height - settings.offset_y
    } else {
        origin.y + settings.offset_y
    };

    let _ = window.set_size(PhysicalSize::new(width as u32, height as u32));
    let _ = window.set_position(PhysicalPosition::new(x, y));
    let _ = window.show();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_url_contains_resolved_profile_and_opacity() {
        let mut config = AppConfig::default();
        config.desktop_overlay.opacity = 0.64;
        config.overlay_profiles[0].show_focus = true;
        let runtime = DesktopOverlayRuntimeConfig::from_app_config(&config);
        let url = overlay_url(&Url::parse("http://127.0.0.1:49321").unwrap(), &runtime);
        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(query.get("profile").map(String::as_str), Some("broadcast"));
        assert_eq!(query.get("bg").map(String::as_str), Some("64"));
        assert!(query
            .get("show")
            .unwrap()
            .split(',')
            .any(|item| item == "focus"));
    }
}
