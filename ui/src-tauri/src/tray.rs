use std::sync::Mutex;

use serde_json::Value;
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};
use tautra_ipc::api::{method, ConnectionStatus, IdleReason, TransferState};

use crate::agent_link::{AgentLink, LinkSnapshot};
use crate::i18n::{t, with, with_name, Key};
use crate::{commands, window};

const ICON_IDLE: &[u8] = include_bytes!("../icons/tray-idle.png");
const ICON_SEARCHING: &[u8] = include_bytes!("../icons/tray-searching.png");
const ICON_CONNECTED: &[u8] = include_bytes!("../icons/tray-connected.png");
const ICON_ERROR: &[u8] = include_bytes!("../icons/tray-error.png");

#[derive(Clone, Copy, PartialEq, Eq)]
enum IconKind {
    Idle,
    Searching,
    Connected,
    Error,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Toggle {
    None,
    Disconnect,
    Resume,
}

struct TrayState {
    tray: TrayIcon,
    status: MenuItem<Wry>,
    open: MenuItem<Wry>,
    toggle: MenuItem<Wry>,
    gaming: CheckMenuItem<Wry>,
    quit: MenuItem<Wry>,
    toggle_action: Mutex<Toggle>,
    icon: Mutex<IconKind>,
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let status = MenuItem::with_id(app, "status", t(Key::AgentOffline), false, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", t(Key::Open), true, None::<&str>)?;
    let toggle = MenuItem::with_id(app, "toggle", t(Key::Disconnect), false, None::<&str>)?;
    let gaming = CheckMenuItem::with_id(app, "gaming", t(Key::Gaming), false, false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", t(Key::Quit), true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &status,
            &PredefinedMenuItem::separator(app)?,
            &open,
            &toggle,
            &gaming,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    let tray = TrayIconBuilder::with_id("tautra")
        .icon(Image::from_bytes(ICON_ERROR)?)
        .tooltip("Tautra")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => window::show_main(app),
            "toggle" => on_toggle(app),
            "gaming" => on_gaming(app),
            "quit" => commands::quit(app.clone()),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                window::show_main(tray.app_handle());
            }
        })
        .build(app)?;

    app.manage(TrayState {
        tray,
        status,
        open,
        toggle,
        gaming,
        quit,
        toggle_action: Mutex::new(Toggle::None),
        icon: Mutex::new(IconKind::Error),
    });
    Ok(())
}

fn describe(snapshot: &LinkSnapshot) -> (String, IconKind, Toggle) {
    let Some(status) = snapshot.status.as_ref().filter(|_| snapshot.connected) else {
        return (t(Key::AgentOffline), IconKind::Error, Toggle::None);
    };
    match &status.connection {
        ConnectionStatus::Idle {
            reason: IdleReason::Starting,
        } => (t(Key::Starting), IconKind::Idle, Toggle::None),
        ConnectionStatus::Idle { .. } => (t(Key::Idle), IconKind::Idle, Toggle::None),
        ConnectionStatus::Searching => (t(Key::Searching), IconKind::Searching, Toggle::None),
        ConnectionStatus::Connecting { device_name, .. } => (
            with_name(Key::Connecting, device_name),
            IconKind::Searching,
            Toggle::Disconnect,
        ),
        ConnectionStatus::Connected(peer) => (
            with_name(Key::Connected, &peer.device_name),
            IconKind::Connected,
            Toggle::Disconnect,
        ),
        ConnectionStatus::Paused => (t(Key::Paused), IconKind::Idle, Toggle::Resume),
        ConnectionStatus::Error { message, .. } => (
            format!("{}: {message}", t(Key::Error)),
            IconKind::Error,
            Toggle::None,
        ),
    }
}

fn transfer_progress(snapshot: &LinkSnapshot) -> Option<u64> {
    let status = snapshot.status.as_ref()?;
    let (done, total) = status
        .share
        .transfers
        .iter()
        .filter(|t| matches!(t.state, TransferState::Running | TransferState::Waiting))
        .fold((0u64, 0u64), |(d, n), t| {
            (d + t.done_bytes.min(t.total_bytes), n + t.total_bytes)
        });
    if total == 0 {
        return None;
    }
    Some(done * 100 / total)
}

pub fn update(app: &AppHandle, snapshot: &LinkSnapshot) {
    let Some(state) = app.try_state::<TrayState>() else {
        return;
    };
    let (label, icon, toggle) = describe(snapshot);
    let _ = state.status.set_text(format!("Tautra: {label}"));
    let tooltip = match transfer_progress(snapshot) {
        Some(percent) => format!(
            "Tautra — {label} · {}",
            with(Key::TrayTransfers, "percent", &format!("{percent}%"))
        ),
        None => format!("Tautra — {label}"),
    };
    let _ = state.tray.set_tooltip(Some(tooltip));
    let _ = state.open.set_text(t(Key::Open));
    let _ = state.quit.set_text(t(Key::Quit));
    let _ = state.toggle.set_text(match toggle {
        Toggle::Resume => t(Key::Resume),
        _ => t(Key::Disconnect),
    });
    let _ = state.toggle.set_enabled(toggle != Toggle::None);
    *state.toggle_action.lock().expect("tray lock") = toggle;

    let kvm = snapshot.status.as_ref().filter(|_| snapshot.connected).map(|s| &s.kvm);
    let _ = state.gaming.set_text(t(Key::Gaming));
    let _ = state.gaming.set_enabled(kvm.is_some());
    let _ = state.gaming.set_checked(kvm.map(|k| k.gaming).unwrap_or(false));

    let mut current = state.icon.lock().expect("tray lock");
    if *current != icon {
        let bytes = match icon {
            IconKind::Idle => ICON_IDLE,
            IconKind::Searching => ICON_SEARCHING,
            IconKind::Connected => ICON_CONNECTED,
            IconKind::Error => ICON_ERROR,
        };
        match Image::from_bytes(bytes) {
            Ok(image) => {
                let _ = state.tray.set_icon(Some(image));
                *current = icon;
            }
            Err(e) => tracing::warn!("ikon tray gagal dimuat: {e}"),
        }
    }
}

fn on_toggle(app: &AppHandle) {
    let Some(state) = app.try_state::<TrayState>() else {
        return;
    };
    let name = match *state.toggle_action.lock().expect("tray lock") {
        Toggle::Disconnect => method::CONNECTION_DISCONNECT,
        Toggle::Resume => method::CONNECTION_RESUME,
        Toggle::None => return,
    };
    let link = app.state::<AgentLink>().inner().clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = link.call(name, Value::Null).await {
            tracing::warn!("perintah tray gagal: {e}");
        }
    });
}

fn on_gaming(app: &AppHandle) {
    let Some(state) = app.try_state::<TrayState>() else {
        return;
    };
    let enabled = state.gaming.is_checked().unwrap_or(false);
    let link = app.state::<AgentLink>().inner().clone();
    tauri::async_runtime::spawn(async move {
        let params = serde_json::json!({ "enabled": enabled });
        if let Err(e) = link.call(method::KVM_SET_GAMING, params).await {
            tracing::warn!("gaming mode dari tray gagal: {e}");
        }
    });
}
