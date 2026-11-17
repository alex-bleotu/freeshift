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

pub async fn run(
    keys: HashMap<Action, Key>,
    hold: Duration,
    focus_prefix: String,
    description: String,
    mut actions: mpsc::UnboundedReceiver<(Action, bool)>,
    events: broadcast::Sender<EngineEvent>,
    mut stop: watch::Receiver<bool>,
) -> Result<()> {
    let _ = events.send(EngineEvent::OutputReady { description });
    let mut pressed_at: HashMap<Action, Instant> = HashMap::new();
    let mut warned_focus = false;
    loop {
        tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    break;
                }
            }
            action = actions.recv() => {
                let Some((action, pressed)) = action else {
                    break;
                };
                let Some(key) = keys.get(&action).copied() else {
                    continue;
                };
                if pressed {
                    if !focus_window(&focus_prefix) && !warned_focus {
                        warned_focus = true;
                        let _ = events.send(EngineEvent::Log {
                            message: format!("No window starting with \"{focus_prefix}\" found; sending keys to the active window"),
                        });
                    }
                    pressed_at.insert(action, Instant::now());
                    if let Err(error) = send_key(key, true) {
                        let _ = events.send(EngineEvent::Log { message: error.to_string() });
                    }
                } else {
                    if let Some(started) = pressed_at.remove(&action) {
                        let elapsed = started.elapsed();
                        if elapsed < hold {
                            tokio::time::sleep(hold - elapsed).await;
                        }
                    }
                    if let Err(error) = send_key(key, false) {
                        let _ = events.send(EngineEvent::Log { message: error.to_string() });
                    }
                }
            }
        }
    }
    for action in pressed_at.keys() {
        if let Some(key) = keys.get(action) {
            let _ = send_key(*key, false);
        }
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
    fn arrows_are_extended() {
        assert_eq!(parse_key("Up"), Some(Key { scan: 0x48, extended: true }));
    }

    #[test]
    fn unknown_names_are_rejected() {
        assert_eq!(parse_key("Hyper"), None);
        assert_eq!(parse_key(""), None);
    }
}
