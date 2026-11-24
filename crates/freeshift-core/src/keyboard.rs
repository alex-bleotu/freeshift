use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use tokio::sync::{broadcast, mpsc, watch};
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP,
    KEYEVENTF_SCANCODE, MAPVK_VK_TO_VSC, MapVirtualKeyW, SendInput, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetWindowTextW, IsWindowVisible, SetForegroundWindow,
};
use windows::core::BOOL;

use crate::config::Config;
use crate::model::{Action, EngineEvent};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub scan: u16,
    pub extended: bool,
}

pub fn parse_key(name: &str) -> Option<Key> {
    let upper = name.trim().to_ascii_uppercase();
    let extended_keys: [(&str, u16); 8] = [
        ("UP", 0x48),
        ("DOWN", 0x50),
        ("LEFT", 0x4B),
        ("RIGHT", 0x4D),
        ("PAGEUP", 0x49),
        ("PAGEDOWN", 0x51),
        ("HOME", 0x47),
        ("END", 0x4F),
    ];
    if let Some((_, scan)) = extended_keys.iter().find(|(label, _)| *label == upper) {
        return Some(Key { scan: *scan, extended: true });
    }
    let virtual_key: u32 = match upper.as_str() {
        "SPACE" => 0x20,
        "ESCAPE" | "ESC" => 0x1B,
        "BACKSPACE" => 0x08,
        "ENTER" => 0x0D,
        "TAB" => 0x09,
        "MINUS" | "-" => 0xBD,
        "PLUS" | "=" => 0xBB,
        "COMMA" | "," => 0xBC,
        "PERIOD" | "." => 0xBE,
        single if single.len() == 1 && single.chars().all(|character| character.is_ascii_alphanumeric()) => {
            u32::from(single.as_bytes()[0])
        }
        function if function.starts_with('F') && function[1..].parse::<u32>().is_ok_and(|number| (1..=12).contains(&number)) => {
            0x6F + function[1..].parse::<u32>().ok()?
        }
        _ => return None,
    };
    let scan = unsafe { MapVirtualKeyW(virtual_key, MAPVK_VK_TO_VSC) };
    let scan = u16::try_from(scan).ok().filter(|value| *value != 0)?;
    Some(Key { scan, extended: false })
}

pub fn send_key(key: Key, down: bool) -> Result<()> {
    let mut flags: KEYBD_EVENT_FLAGS = KEYEVENTF_SCANCODE;
    if key.extended {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    if !down {
        flags |= KEYEVENTF_KEYUP;
    }
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0),
                wScan: key.scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let sent = unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
    if sent != 1 {
        bail!("Windows blocked the key press (is the game running as administrator?)");
    }
    Ok(())
}

struct WindowSearch {
    prefix: String,
    found: Option<HWND>,
}

unsafe extern "system" fn match_window(window: HWND, parameter: LPARAM) -> BOOL {
    let search = unsafe { &mut *(parameter.0 as *mut WindowSearch) };
    if !unsafe { IsWindowVisible(window) }.as_bool() {
        return BOOL(1);
    }
    let mut buffer = [0u16; 256];
    let length = unsafe { GetWindowTextW(window, &mut buffer) };
    if length <= 0 {
        return BOOL(1);
    }
    let title = String::from_utf16_lossy(&buffer[..length as usize]).to_lowercase();
    if title.starts_with(&search.prefix) {
        search.found = Some(window);
        return BOOL(0);
    }
    BOOL(1)
}

pub fn focus_window(prefix: &str) -> bool {
    if prefix.trim().is_empty() {
        return true;
    }
    let mut search = WindowSearch {
        prefix: prefix.trim().to_lowercase(),
        found: None,
    };
    let _ = unsafe { EnumWindows(Some(match_window), LPARAM(&mut search as *mut WindowSearch as isize)) };
    let Some(window) = search.found else {
        return false;
    };
    if unsafe { GetForegroundWindow() } == window {
        return true;
    }
    unsafe { SetForegroundWindow(window) }.as_bool()
}

#[derive(Debug, PartialEq, Eq)]
struct Bindings {
    keys: HashMap<Action, Key>,
    hold: Duration,
    focus_prefix: String,
    description: String,
    unknown: Vec<String>,
}

impl Bindings {
    fn from_config(config: &Config) -> Bindings {
        let mut keys = HashMap::new();
        let mut unknown = Vec::new();
        for (action, name) in &config.keys {
            if name.trim().is_empty() {
                continue;
            }
            match parse_key(name) {
                Some(key) => {
                    keys.insert(*action, key);
                }
                None => unknown.push(format!("Unknown key \"{name}\" for {}", action.label())),
            }
        }
        let focus_prefix = config.focus_window_prefix.clone();
        let target = if focus_prefix.trim().is_empty() {
            "the active window".to_string()
        } else {
            format!("the \"{focus_prefix}\" window")
        };
        let key_of = |action: Action| config.keys.get(&action).cloned().unwrap_or_else(|| "none".to_string());
        let description = format!(
            "Keyboard output: {} shifts up, {} shifts down, {} keys in total, sent to {target}",
            key_of(Action::ShiftUp),
            key_of(Action::ShiftDown),
            keys.len()
        );
        Bindings {
            keys,
            hold: Duration::from_millis(config.key_hold_ms),
            focus_prefix,
            description,
            unknown,
        }
    }

    fn announce(&self, events: &broadcast::Sender<EngineEvent>) {
        for message in &self.unknown {
            let _ = events.send(EngineEvent::Log { message: message.clone() });
        }
        let _ = events.send(EngineEvent::OutputReady { description: self.description.clone() });
    }
}

pub async fn run(
    mut settings: watch::Receiver<Config>,
    mut actions: mpsc::UnboundedReceiver<(Action, bool)>,
    events: broadcast::Sender<EngineEvent>,
    mut stop: watch::Receiver<bool>,
) -> Result<()> {
    let mut bindings = Bindings::from_config(&settings.borrow_and_update());
    bindings.announce(&events);
    let mut held: HashMap<Action, (Key, Instant)> = HashMap::new();
    let mut warned_focus = false;
    loop {
        tokio::select! {
            biased;
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    break;
                }
            }
            Ok(()) = settings.changed() => {
                let next = Bindings::from_config(&settings.borrow_and_update());
                if next != bindings {
                    warned_focus = warned_focus && next.focus_prefix == bindings.focus_prefix;
                    bindings = next;
                    bindings.announce(&events);
                }
            }
            action = actions.recv() => {
                let Some((action, pressed)) = action else {
                    break;
                };
                if pressed {
                    let Some(key) = bindings.keys.get(&action).copied() else {
                        continue;
                    };
                    if !focus_window(&bindings.focus_prefix) && !warned_focus {
                        warned_focus = true;
                        let _ = events.send(EngineEvent::Log {
                            message: format!(
                                "No window starting with \"{}\" found; sending keys to the active window",
                                bindings.focus_prefix
                            ),
                        });
                    }
                    held.insert(action, (key, Instant::now()));
                    if let Err(error) = send_key(key, true) {
                        let _ = events.send(EngineEvent::Log { message: error.to_string() });
                    }
                } else {
                    let Some((key, started)) = held.remove(&action) else {
                        continue;
                    };
                    let elapsed = started.elapsed();
                    if elapsed < bindings.hold {
                        tokio::time::sleep(bindings.hold - elapsed).await;
                    }
                    if let Err(error) = send_key(key, false) {
                        let _ = events.send(EngineEvent::Log { message: error.to_string() });
                    }
                }
            }
        }
    }
    for (key, _) in held.values() {
        let _ = send_key(*key, false);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_map_to_scan_codes() {
        assert_eq!(parse_key("i"), Some(Key { scan: 0x17, extended: false }));
        assert_eq!(parse_key("K"), Some(Key { scan: 0x25, extended: false }));
    }

    #[test]
    fn escape_and_tab_are_known() {
        assert_eq!(parse_key("Escape"), Some(Key { scan: 0x01, extended: false }));
        assert_eq!(parse_key("tab"), Some(Key { scan: 0x0F, extended: false }));
    }

    #[test]
    fn arrows_are_extended() {
        assert_eq!(parse_key("Up"), Some(Key { scan: 0x48, extended: true }));
    }

    #[test]
    fn bindings_follow_the_config() {
        let mut config = Config::default();
        let before = Bindings::from_config(&config);
        config.mapping.clear();
        assert_eq!(Bindings::from_config(&config), before);

        config.keys.insert(Action::ShiftUp, "P".to_string());
        config.keys.insert(Action::ShiftDown, "Hyper".to_string());
        config.key_hold_ms = 120;
        let after = Bindings::from_config(&config);
        assert_eq!(after.keys.get(&Action::ShiftUp).copied(), parse_key("P"));
        assert_eq!(after.keys.get(&Action::ShiftDown), None);
        assert_eq!(after.hold, Duration::from_millis(120));
        assert_eq!(after.unknown.len(), 1);
        assert!(after.description.contains("P shifts up"));
    }

    #[tokio::test]
    async fn announces_again_only_when_its_own_settings_change() {
        let (settings, settings_receiver) = watch::channel(Config::default());
        let (_actions, action_receiver) = mpsc::unbounded_channel();
        let (events, mut received) = broadcast::channel(16);
        let (stop, stop_receiver) = watch::channel(false);
        let task = tokio::spawn(run(settings_receiver, action_receiver, events, stop_receiver));
        assert!(matches!(received.recv().await, Ok(EngineEvent::OutputReady { .. })));

        settings.send_modify(|config| config.mapping.clear());
        tokio::task::yield_now().await;
        settings.send_modify(|config| {
            config.keys.insert(Action::ShiftUp, "P".to_string());
        });
        let Ok(EngineEvent::OutputReady { description }) = received.recv().await else {
            panic!("expected the new description");
        };
        assert!(description.contains("P shifts up"));
        stop.send(true).unwrap();
        task.await.unwrap().unwrap();
        assert!(received.try_recv().is_err());
    }

    #[test]
    fn unknown_names_are_rejected() {
        assert_eq!(parse_key("Hyper"), None);
        assert_eq!(parse_key(""), None);
    }
}
