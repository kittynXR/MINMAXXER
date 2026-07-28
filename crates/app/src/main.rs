#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio_output;
mod boss_alert;
mod config;
mod desktop_overlay;
mod server;
mod storage;
mod tailer;
mod vr_overlay;

use crate::config::{app_data_directory, AppConfig};
use crate::desktop_overlay::{
    spawn_desktop_overlay, DesktopOverlayHandle, DesktopOverlayRuntimeConfig,
};
use crate::server::ServerState;
use crate::storage::Storage;
use crate::tailer::{spawn_collector, CollectorHandle, CollectorSettings};
use crate::vr_overlay::{spawn_vr_overlay, VrOverlayHandle, VrOverlaySettings, VrOverlayStatus};
use anyhow::{Context, Result};
use minmaxxer_core::EngineSnapshot;
use std::net::{Ipv4Addr, TcpListener};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::webview::{Color, PageLoadEvent};
use tauri::{Manager, Theme, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tokio::sync::watch;
use tracing_subscriber::EnvFilter;
use url::Url;

struct RuntimeHandles {
    collector: CollectorHandle,
    desktop: DesktopOverlayHandle,
    vr: Mutex<Option<VrOverlayHandle>>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("MINMAXXER could not start: {error:#}");
        #[cfg(windows)]
        show_startup_error(&format!("MINMAXXER could not start:\n\n{error:#}"));
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    initialize_logging();
    let data_directory = app_data_directory();
    let config_path = data_directory.join("config.json");
    let database_path = data_directory.join("combat.sqlite");
    let imports_directory = data_directory.join("imports");
    let config = AppConfig::load(&config_path)?;
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, config.port)).with_context(|| {
        format!(
            "local port {} is already in use; change `port` in {} or close the other MINMAXXER instance",
            config.port,
            config_path.display()
        )
    })?;
    listener.set_nonblocking(true)?;

    let storage = Arc::new(Storage::open(&database_path)?);
    let (snapshot_tx, snapshot_rx) = watch::channel(EngineSnapshot::default());
    let (boss_target_tx, boss_target_rx) = tokio::sync::mpsc::unbounded_channel();
    let collector = spawn_collector(
        CollectorSettings::from(&config),
        storage.clone(),
        snapshot_tx,
        boss_target_tx,
    );
    let shared_config = Arc::new(RwLock::new(config.clone()));
    let (config_tx, config_rx) = watch::channel(config.clone());

    if std::env::args().any(|argument| argument == "--headless") {
        return run_headless(
            listener,
            snapshot_rx,
            boss_target_rx,
            storage,
            shared_config,
            config_path,
            config_tx,
            collector,
            imports_directory,
        );
    }

    let config_for_setup = config.clone();
    let shared_config_for_setup = shared_config.clone();
    let storage_for_setup = storage.clone();
    let collector_for_setup = collector.clone();
    let config_path_for_setup = config_path.clone();
    let imports_for_setup = imports_directory.clone();
    let snapshots_for_setup = snapshot_rx.clone();
    let boss_targets_for_setup = boss_target_rx;
    let updates_for_setup = config_tx.clone();

    tauri::Builder::default()
        .setup(move |app| {
            app.set_theme(Some(Theme::Dark));
            let (vr_api_tx, vr_api_rx) = watch::channel(VrOverlayStatus::default());
            let server_state = ServerState {
                snapshots: snapshots_for_setup.clone(),
                storage: storage_for_setup.clone(),
                config: shared_config_for_setup.clone(),
                config_path: config_path_for_setup.clone(),
                config_updates: updates_for_setup.clone(),
                collector: collector_for_setup.clone(),
                imports_directory: imports_for_setup.clone(),
                started_at: Instant::now(),
                vr_status: Some(vr_api_rx),
            };
            tauri::async_runtime::spawn(async move {
                if let Err(error) = server::serve_on(server_state, listener).await {
                    tracing::error!(%error, "local HTTP/OBS server stopped");
                }
            });
            tauri::async_runtime::spawn(boss_alert::monitor(
                boss_targets_for_setup,
                shared_config_for_setup.clone(),
            ));

            let origin = format!("http://127.0.0.1:{}", config_for_setup.port);
            let main_url = Url::parse(&origin)?;
            let reveal_main_window = Arc::new(AtomicBool::new(!config_for_setup.launch_minimized));
            let reveal_on_load = reveal_main_window.clone();
            let _main_window =
                WebviewWindowBuilder::new(app, "main", WebviewUrl::External(main_url))
                    .title("MINMAXXER — Ecliptica Combat Lab")
                    .inner_size(1440.0, 900.0)
                    .min_inner_size(1000.0, 680.0)
                    .center()
                    .theme(Some(Theme::Dark))
                    .background_color(Color(8, 11, 17, 255))
                    .visible(false)
                    .on_page_load(move |window, payload| {
                        if matches!(payload.event(), PageLoadEvent::Finished)
                            && reveal_on_load.swap(false, Ordering::AcqRel)
                        {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    })
                    .build()?;

            let (desktop_tx, desktop_rx) = watch::channel(
                DesktopOverlayRuntimeConfig::from_app_config(&config_for_setup),
            );
            let (vr_tx, vr_rx) = watch::channel(config_for_setup.vr_overlay);
            spawn_config_fanout(config_rx.clone(), desktop_tx, vr_tx);

            let desktop =
                spawn_desktop_overlay(app.handle().clone(), Url::parse(&origin)?, desktop_rx)?;
            let vr = spawn_vr_overlay(snapshots_for_setup.clone(), vr_rx);
            let vr_status = vr.subscribe_status();
            vr_api_tx.send_replace(vr_status.borrow().clone());
            let mut vr_status_updates = vr_status.clone();
            tauri::async_runtime::spawn(async move {
                while vr_status_updates.changed().await.is_ok() {
                    vr_api_tx.send_replace(vr_status_updates.borrow_and_update().clone());
                }
            });
            app.manage(vr_status);

            let tray = build_tray(app)?;
            app.manage(tray);
            app.manage(RuntimeHandles {
                collector: collector_for_setup.clone(),
                desktop,
                vr: Mutex::new(Some(vr)),
            });
            app.manage(shared_config_for_setup.clone());
            Ok(())
        })
        .on_window_event(move |window, event| {
            if window.label() != "main" {
                return;
            }
            if let WindowEvent::CloseRequested { api, .. } = event {
                let minimize_to_tray = shared_config
                    .read()
                    .map(|config| config.minimize_to_tray)
                    .unwrap_or(true);
                if minimize_to_tray {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .build(tauri::generate_context!())?
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::ExitRequested { .. }) {
                if let Some(handles) = app.try_state::<RuntimeHandles>() {
                    handles.collector.stop();
                    handles.desktop.stop();
                    if let Ok(mut vr) = handles.vr.lock() {
                        if let Some(vr) = vr.as_mut() {
                            vr.request_shutdown();
                        }
                    }
                }
            }
        });
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_headless(
    listener: TcpListener,
    snapshots: watch::Receiver<EngineSnapshot>,
    boss_targets: tokio::sync::mpsc::UnboundedReceiver<boss_alert::BossTargetUpdate>,
    storage: Arc<Storage>,
    config: Arc<RwLock<AppConfig>>,
    config_path: std::path::PathBuf,
    config_updates: watch::Sender<AppConfig>,
    collector: CollectorHandle,
    imports_directory: std::path::PathBuf,
) -> Result<()> {
    tracing::info!("running without embedded WebView; OBS and API remain available");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(2)
        .thread_name("minmaxxer-runtime")
        .build()?;
    runtime.spawn(boss_alert::monitor(boss_targets, config.clone()));
    runtime.block_on(server::serve_on(
        ServerState {
            snapshots,
            storage,
            config,
            config_path,
            config_updates,
            collector,
            imports_directory,
            started_at: Instant::now(),
            vr_status: None,
        },
        listener,
    ))
}

fn spawn_config_fanout(
    mut config_rx: watch::Receiver<AppConfig>,
    desktop_tx: watch::Sender<DesktopOverlayRuntimeConfig>,
    vr_tx: watch::Sender<VrOverlaySettings>,
) {
    tauri::async_runtime::spawn(async move {
        while config_rx.changed().await.is_ok() {
            let config = config_rx.borrow_and_update().clone();
            let _ = desktop_tx.send(DesktopOverlayRuntimeConfig::from_app_config(&config));
            let _ = vr_tx.send(config.vr_overlay);
        }
    });
}

fn build_tray(app: &tauri::App) -> tauri::Result<tauri::tray::TrayIcon> {
    let show = MenuItem::with_id(app, "show", "Open Combat Lab", true, None::<&str>)?;
    let overlay = MenuItem::with_id(app, "overlay", "Overlay Studio", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit MINMAXXER", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &overlay, &quit])?;
    TrayIconBuilder::new()
        .tooltip("MINMAXXER — watching VRChat")
        .icon(tray_icon())
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_main(app, None),
            "overlay" => show_main(app, Some("overlay")),
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)
}

fn show_main(app: &tauri::AppHandle, view: Option<&str>) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if let Some(view) = view {
        let port = app
            .state::<Arc<RwLock<AppConfig>>>()
            .read()
            .map(|config| config.port)
            .unwrap_or(config::DEFAULT_PORT);
        if let Ok(url) = Url::parse(&format!("http://127.0.0.1:{port}/?view={view}")) {
            let _ = window.navigate(url);
        }
    }
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
}

fn tray_icon() -> Image<'static> {
    let width = 32;
    let height = 32;
    let mut rgba = vec![0_u8; width * height * 4];
    for y in 0..height {
        for x in 0..width {
            let index = (y * width + x) * 4;
            let dx = x as i32 - 16;
            let dy = y as i32 - 16;
            let inside = dx * dx + dy * dy <= 15 * 15;
            if inside {
                rgba[index] = 11;
                rgba[index + 1] = 18;
                rgba[index + 2] = 34;
                rgba[index + 3] = 255;
            }
            let left_m = (7..=11).contains(&x) && (8..=24).contains(&y);
            let right_m = (21..=25).contains(&x) && (8..=24).contains(&y);
            let diagonal = (x as i32 - 16).abs() <= 2 && (10..=20).contains(&y);
            if inside && (left_m || right_m || diagonal) {
                rgba[index] = 114;
                rgba[index + 1] = 239;
                rgba[index + 2] = 207;
                rgba[index + 3] = 255;
            }
        }
    }
    Image::new_owned(rgba, width as u32, height as u32)
}

fn initialize_logging() {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("minmaxxer=info,warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .try_init();
}

#[cfg(windows)]
fn show_startup_error(message: &str) {
    use windows::core::{w, HSTRING};
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
    unsafe {
        let _ = MessageBoxW(
            None,
            &HSTRING::from(message),
            w!("MINMAXXER"),
            MB_OK | MB_ICONERROR,
        );
    }
}
