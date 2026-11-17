#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;

use freeshift_core::engine::{self, EngineHandle};
use freeshift_core::{Action, Config, EngineEvent};
use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{App, AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tokio::sync::broadcast;

const LOG_LIMIT: usize = 300;
const MINIMIZED_FLAG: &str = "--minimized";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceInfo {
    id: String,
    name: String,
    kind: String,
    battery: Option<u8>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TrainerAppInfo {
    peer: String,
    app: String,
    version: String,
    supported: Vec<Action>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ActionInfo {
    id: Action,
    label: &'static str,
}

#[derive(Default)]
struct Status {
    devices: BTreeMap<String, DeviceInfo>,
    clients: Vec<String>,
    trainer_apps: BTreeMap<String, TrainerAppInfo>,
    output: String,
    log: VecDeque<String>,
}

impl Status {
    fn push_log(&mut self, line: String) {
        self.log.push_back(line);
        while self.log.len() > LOG_LIMIT {
            self.log.pop_front();
        }
    }

    fn apply(&mut self, event: &EngineEvent) {
        match event {
            EngineEvent::Log { message } => self.push_log(message.clone()),
            EngineEvent::DeviceConnected { id, name, kind } => {
                self.push_log(format!("Connected {name} ({kind})"));
                self.devices.insert(
                    id.clone(),
                    DeviceInfo {
                        id: id.clone(),
                        name: name.clone(),
                        kind: kind.clone(),
                        battery: None,
                    },
                );
            }
            EngineEvent::DeviceDisconnected { id, name } => {
                self.push_log(format!("Disconnected {name}"));
                self.devices.remove(id);
            }
            EngineEvent::Battery { id, level } => {
                if let Some(device) = self.devices.get_mut(id) {
                    device.battery = Some(*level);
                }
            }
            EngineEvent::Unlocked { id } => self.push_log(format!("{id} answered the handshake")),
            EngineEvent::OutputClient { peer, connected } => {
                if *connected {
                    self.push_log(format!("Trainer app connected from {peer}"));
                    self.clients.push(peer.clone());
                } else {
                    self.clients.retain(|client| client != peer);
                    self.trainer_apps.remove(peer);
                }
            }
            EngineEvent::TrainerApp { peer, app, version, supported } => {
                self.push_log(format!("{peer} is {app} {version}"));
                self.trainer_apps.insert(
                    peer.clone(),
                    TrainerAppInfo {
                        peer: peer.clone(),
                        app: app.clone(),
                        version: version.clone(),
                        supported: supported.clone(),
                    },
                );
            }
            EngineEvent::OutputReady { description } => {
                self.push_log(description.clone());
                self.output = description.clone();
            }
            EngineEvent::Notification { device, characteristic, hex } => {
                self.push_log(format!("{device} {characteristic}: {hex}"));
            }
            EngineEvent::Button { .. } | EngineEvent::Action { .. } => {}
        }
    }
}

struct Shared {
    events: broadcast::Sender<EngineEvent>,
    engine: tokio::sync::Mutex<Option<EngineHandle>>,
    status: Mutex<Status>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    config: Config,
    devices: Vec<DeviceInfo>,
    clients: Vec<String>,
    trainer_apps: Vec<TrainerAppInfo>,
    actions: Vec<ActionInfo>,
    output: String,
    log: Vec<String>,
    autostart: bool,
    config_path: String,
}

fn describe(error: impl std::fmt::Display) -> String {
    format!("{error:#}")
}

#[tauri::command]
fn get_snapshot(app: AppHandle, shared: State<'_, Shared>) -> Result<Snapshot, String> {
    let config = Config::load().map_err(describe)?;
    let config_path = Config::path().map(|path| path.display().to_string()).unwrap_or_default();
    let status = shared.status.lock().map_err(describe)?;
    Ok(Snapshot {
        config,
        devices: status.devices.values().cloned().collect(),
        clients: status.clients.clone(),
        trainer_apps: status.trainer_apps.values().cloned().collect(),
        actions: Action::ALL.iter().map(|action| ActionInfo { id: *action, label: action.label() }).collect(),
        output: status.output.clone(),
        log: status.log.iter().cloned().collect(),
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        config_path,
    })
}

#[tauri::command]
async fn save_config(app: AppHandle, config: Config, shared: State<'_, Shared>) -> Result<(), String> {
    config.save().map_err(describe)?;
    restart(&app, &shared, config).await;
    Ok(())
}

#[tauri::command]
async fn swap_directions(app: AppHandle, shared: State<'_, Shared>) -> Result<Config, String> {
    let mut config = Config::load().map_err(describe)?;
    config.swap_directions();
    config.save().map_err(describe)?;
    restart(&app, &shared, config.clone()).await;
    Ok(config)
}

#[tauri::command]
fn set_autostart(app: AppHandle, enabled: bool) -> Result<bool, String> {
    let manager = app.autolaunch();
    let result = if enabled { manager.enable() } else { manager.disable() };
    result.map_err(describe)?;
    manager.is_enabled().map_err(describe)
}

async fn restart(app: &AppHandle, shared: &Shared, config: Config) {
    let mut engine = shared.engine.lock().await;
    if let Some(handle) = engine.take() {
        handle.stop().await;
    }
    if let Ok(mut status) = shared.status.lock() {
        status.devices.clear();
        status.clients.clear();
        status.trainer_apps.clear();
        status.output.clear();
    }
    *engine = Some(engine::start(config, shared.events.clone()));
    let _ = app.emit("engine-restarted", ());
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn quit(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let shared = app.state::<Shared>();
        if let Some(handle) = shared.engine.lock().await.take() {
            handle.stop().await;
        }
        app.exit(0);
    });
}

fn build_tray(app: &App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Open FreeShift", true, None::<&str>)?;
    let exit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &exit])?;
    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("FreeShift")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main_window(app),
            "quit" => quit(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

fn forward_events(app: AppHandle, events: &broadcast::Sender<EngineEvent>) {
    let mut receiver = events.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            match receiver.recv().await {
                Ok(event) => {
                    let shared = app.state::<Shared>();
                    if let Ok(mut status) = shared.status.lock() {
                        status.apply(&event);
                    }
                    let _ = app.emit("engine-event", &event);
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

fn main() {
    let (events, _) = broadcast::channel(512);
    let shared = Shared {
        events: events.clone(),
        engine: tokio::sync::Mutex::new(None),
        status: Mutex::new(Status::default()),
    };
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _arguments, _directory| show_main_window(app)))
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec![MINIMIZED_FLAG])))
        .manage(shared)
        .invoke_handler(tauri::generate_handler![get_snapshot, save_config, swap_directions, set_autostart])
        .setup(move |app| {
            build_tray(app)?;
            let handle = app.handle().clone();
            forward_events(handle.clone(), &events);
            if !std::env::args().any(|argument| argument == MINIMIZED_FLAG) {
                show_main_window(&handle);
            }
            tauri::async_runtime::spawn(async move {
                let shared = handle.state::<Shared>();
                let config = match Config::load() {
                    Ok(config) => config,
                    Err(error) => {
                        let _ = shared.events.send(EngineEvent::Log {
                            message: format!("Could not read settings, using defaults: {error:#}"),
                        });
                        Config::default()
                    }
                };
                restart(&handle, &shared, config).await;
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!());
    if let Err(error) = result {
        eprintln!("FreeShift failed to start: {error}");
        std::process::exit(1);
    }
}
