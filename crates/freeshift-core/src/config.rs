use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::model::{Action, Button};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputMode {
    #[default]
    OpenBikeControl,
    Keyboard,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CustomTrigger {
    pub name_prefix: String,
    pub characteristic: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    pub output: OutputMode,
    pub mapping: BTreeMap<Button, Action>,
    pub controller_name: String,
    pub obc_port: u16,
    pub keys: BTreeMap<Action, String>,
    pub key_hold_ms: u64,
    pub focus_window_prefix: String,
    pub zwift_enabled: bool,
    pub custom_triggers: Vec<CustomTrigger>,
    pub double_tap_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        let mapping = BTreeMap::from([
            (Button::Plus, Action::ShiftUp),
            (Button::Minus, Action::ShiftDown),
            (Button::Up, Action::Up),
            (Button::Down, Action::Down),
            (Button::Left, Action::SteerLeft),
            (Button::Right, Action::SteerRight),
            (Button::A, Action::Select),
            (Button::B, Action::ShiftDown),
            (Button::Y, Action::Menu),
            (Button::Z, Action::Back),
            (Button::Tap, Action::ShiftUp),
            (Button::DoubleTap, Action::ShiftDown),
        ]);
        Config {
            output: OutputMode::OpenBikeControl,
            mapping,
            controller_name: "FreeShift".to_string(),
            obc_port: 21587,
            keys: default_keys(),
            key_hold_ms: 40,
            focus_window_prefix: "MyWhoosh".to_string(),
            zwift_enabled: true,
            custom_triggers: Vec::new(),
            double_tap_ms: 350,
        }
    }
}

pub fn default_keys() -> BTreeMap<Action, String> {
    [
        (Action::ShiftUp, "I"),
        (Action::ShiftDown, "K"),
        (Action::Up, "Up"),
        (Action::Down, "Down"),
        (Action::Left, "Left"),
        (Action::Right, "Right"),
        (Action::Select, "Enter"),
        (Action::Back, "Escape"),
        (Action::Menu, "Escape"),
        (Action::SteerLeft, "A"),
        (Action::SteerRight, "D"),
        (Action::Emote, "1"),
        (Action::UTurn, "U"),
        (Action::CameraView, "Tab"),
        (Action::HudToggle, "H"),
        (Action::PowerUp, "Space"),
    ]
    .into_iter()
    .map(|(action, key)| (action, key.to_string()))
    .collect()
}

impl Config {
    pub fn path() -> Result<PathBuf> {
        let base = dirs::config_dir().context("no config directory on this system")?;
        Ok(base.join("FreeShift").join("config.json"))
    }

    pub fn load() -> Result<Config> {
        let path = Config::path()?;
        if !path.exists() {
            return Ok(Config::default());
        }
        let text = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn save(&self) -> Result<PathBuf> {
        let path = Config::path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        let text = serde_json::to_string_pretty(self)?;
        fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
        Ok(path)
    }

    pub fn swap_directions(&mut self) {
        for action in self.mapping.values_mut() {
            *action = action.swapped();
        }
    }

    pub fn action_for(&self, button: Button) -> Action {
        self.mapping.get(&button).copied().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_json() {
        let config = Config::default();
        let text = serde_json::to_string(&config).unwrap();
        let parsed: Config = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed, config);
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let parsed: Config = serde_json::from_str(r#"{"output":"Keyboard"}"#).unwrap();
        assert_eq!(parsed.output, OutputMode::Keyboard);
        assert_eq!(parsed.keys.get(&Action::ShiftUp).map(String::as_str), Some("I"));
        assert_eq!(parsed.action_for(Button::Plus), Action::ShiftUp);
    }

    #[test]
    fn swap_exchanges_up_and_down() {
        let mut config = Config::default();
        config.swap_directions();
        assert_eq!(config.action_for(Button::Plus), Action::ShiftDown);
        assert_eq!(config.action_for(Button::Minus), Action::ShiftUp);
        assert_eq!(config.action_for(Button::A), Action::Select);
        assert_eq!(config.action_for(Button::Left), Action::SteerLeft);
    }
}
