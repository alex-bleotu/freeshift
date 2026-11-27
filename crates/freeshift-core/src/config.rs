use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer, Serialize};

use crate::model::{Action, Button};

const SRAM_BUTTONS: [Button; 2] = [Button::SramLeft, Button::SramRight];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputMode {
    #[default]
    OpenBikeControl,
    Keyboard,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    pub output: OutputMode,
    #[serde(deserialize_with = "known_buttons")]
    pub mapping: BTreeMap<Button, Action>,
    pub controller_name: String,
    pub obc_port: u16,
    pub keys: BTreeMap<Action, String>,
    pub key_hold_ms: u64,
    pub focus_window_prefix: String,
    pub zwift_enabled: bool,
}

fn known_buttons<'de, D: Deserializer<'de>>(deserializer: D) -> Result<BTreeMap<Button, Action>, D::Error> {
    let mapping = BTreeMap::<String, Action>::deserialize(deserializer)?;
    Ok(mapping
        .into_iter()
        .filter_map(|(label, action)| Button::from_label(&label).map(|button| (button, action)))
        .collect())
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
            (Button::SramLeft, Action::ShiftDown),
            (Button::SramRight, Action::ShiftUp),
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
        let mut config: Config = serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        config.add_missing_sram_mapping();
        Ok(config)
    }

    fn add_missing_sram_mapping(&mut self) {
        let has_sram = SRAM_BUTTONS.iter().any(|button| self.mapping.contains_key(button));
        if has_sram {
            return;
        }
        let defaults = Config::default().mapping;
        for button in SRAM_BUTTONS {
            if let Some(action) = defaults.get(&button) {
                self.mapping.insert(button, *action);
            }
        }
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

    pub fn can_apply_live(&self, next: &Config) -> bool {
        let live = Config {
            mapping: next.mapping.clone(),
            keys: next.keys.clone(),
            key_hold_ms: next.key_hold_ms,
            focus_window_prefix: next.focus_window_prefix.clone(),
            ..self.clone()
        };
        live == *next
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
    fn older_configs_get_the_sram_paddles() {
        let mut config: Config = serde_json::from_str(r#"{"mapping":{"Plus":"ShiftUp"}}"#).unwrap();
        config.add_missing_sram_mapping();
        assert_eq!(config.action_for(Button::SramLeft), Action::ShiftDown);
        assert_eq!(config.action_for(Button::SramRight), Action::ShiftUp);
        assert_eq!(config.action_for(Button::Minus), Action::None);
    }

    #[test]
    fn cleared_sram_paddles_stay_cleared() {
        let mut config: Config = serde_json::from_str(r#"{"mapping":{"SramRight":"Pause"}}"#).unwrap();
        config.add_missing_sram_mapping();
        assert_eq!(config.action_for(Button::SramLeft), Action::None);
        assert_eq!(config.action_for(Button::SramRight), Action::Pause);
    }

    #[test]
    fn older_configs_drop_removed_buttons_and_settings() {
        let text = r#"{"mapping":{"Plus":"ShiftUp","Tap":"ShiftUp","SramLeftDouble":"Pause"},"customTriggers":[{"namePrefix":"X","characteristic":"y"}],"doubleTapMs":350}"#;
        let config: Config = serde_json::from_str(text).unwrap();
        assert_eq!(config.mapping, BTreeMap::from([(Button::Plus, Action::ShiftUp)]));
    }

    #[test]
    fn button_and_key_changes_apply_live() {
        let current = Config::default();
        let mut next = current.clone();
        next.mapping.insert(Button::Plus, Action::Pause);
        next.mapping.insert(Button::SramLeft, Action::Pause);
        next.keys.insert(Action::ShiftUp, "P".to_string());
        next.key_hold_ms = 90;
        next.focus_window_prefix = "Zwift".to_string();
        assert!(current.can_apply_live(&next));
    }

    #[test]
    fn connection_changes_need_a_restart() {
        let current = Config::default();
        let changed = [
            Config { output: OutputMode::Keyboard, ..current.clone() },
            Config { controller_name: "Other".to_string(), ..current.clone() },
            Config { obc_port: 21588, ..current.clone() },
            Config { zwift_enabled: false, ..current.clone() },
        ];
        for next in changed {
            assert!(!current.can_apply_live(&next));
        }
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
