use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Button {
    Plus,
    Minus,
    Left,
    Up,
    Right,
    Down,
    A,
    B,
    Y,
    Z,
    Tap,
    DoubleTap,
}

impl Button {
    pub const ALL: [Button; 12] = [
        Button::Plus,
        Button::Minus,
        Button::Left,
        Button::Up,
        Button::Right,
        Button::Down,
        Button::A,
        Button::B,
        Button::Y,
        Button::Z,
        Button::Tap,
        Button::DoubleTap,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Button::Plus => "Plus",
            Button::Minus => "Minus",
            Button::Left => "Left",
            Button::Up => "Up",
            Button::Right => "Right",
            Button::Down => "Down",
            Button::A => "A",
            Button::B => "B",
            Button::Y => "Y",
            Button::Z => "Z",
            Button::Tap => "Tap",
            Button::DoubleTap => "DoubleTap",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Action {
    ShiftUp,
    ShiftDown,
    Up,
    Down,
    Left,
    Right,
    Select,
    Back,
    Menu,
    Home,
    SteerLeft,
    SteerRight,
    Emote,
    IncreaseDifficulty,
    DecreaseDifficulty,
    SkipInterval,
    Pause,
    Resume,
    UTurn,
    CameraView,
    HudToggle,
    MapToggle,
    PowerUp,
    #[default]
    None,
}

impl Action {
    pub const ALL: [Action; 23] = [
        Action::ShiftUp,
        Action::ShiftDown,
        Action::Up,
        Action::Down,
        Action::Left,
        Action::Right,
        Action::Select,
        Action::Back,
        Action::Menu,
        Action::Home,
        Action::SteerLeft,
        Action::SteerRight,
        Action::Emote,
        Action::IncreaseDifficulty,
        Action::DecreaseDifficulty,
        Action::SkipInterval,
        Action::Pause,
        Action::Resume,
        Action::UTurn,
        Action::CameraView,
        Action::HudToggle,
        Action::MapToggle,
        Action::PowerUp,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Action::ShiftUp => "Shift up",
            Action::ShiftDown => "Shift down",
            Action::Up => "Up",
            Action::Down => "Down",
            Action::Left => "Left",
            Action::Right => "Right",
            Action::Select => "OK / select",
            Action::Back => "Back",
            Action::Menu => "Menu",
            Action::Home => "Home",
            Action::SteerLeft => "Steer left",
            Action::SteerRight => "Steer right",
            Action::Emote => "Emote",
            Action::IncreaseDifficulty => "Harder",
            Action::DecreaseDifficulty => "Easier",
            Action::SkipInterval => "Skip interval",
            Action::Pause => "Pause",
            Action::Resume => "Resume",
            Action::UTurn => "U-turn",
            Action::CameraView => "Camera view",
            Action::HudToggle => "Show / hide HUD",
            Action::MapToggle => "Show / hide map",
            Action::PowerUp => "Power-up",
            Action::None => "Nothing",
        }
    }

    pub fn obc_id(self) -> Option<u8> {
        match self {
            Action::ShiftUp => Some(0x01),
            Action::ShiftDown => Some(0x02),
            Action::Up => Some(0x10),
            Action::Down => Some(0x11),
            Action::Left => Some(0x12),
            Action::Right => Some(0x13),
            Action::Select => Some(0x14),
            Action::Back => Some(0x15),
            Action::Menu => Some(0x16),
            Action::Home => Some(0x17),
            Action::SteerLeft => Some(0x18),
            Action::SteerRight => Some(0x19),
            Action::Emote => Some(0x20),
            Action::IncreaseDifficulty => Some(0x30),
            Action::DecreaseDifficulty => Some(0x31),
            Action::SkipInterval => Some(0x32),
            Action::Pause => Some(0x33),
            Action::Resume => Some(0x34),
            Action::UTurn => Some(0x37),
            Action::CameraView => Some(0x40),
            Action::HudToggle => Some(0x44),
            Action::MapToggle => Some(0x45),
            Action::PowerUp => Some(0x50),
            Action::None => None,
        }
    }

    pub fn from_obc_id(id: u8) -> Option<Action> {
        match id {
            0x01 => Some(Action::ShiftUp),
            0x02 => Some(Action::ShiftDown),
            0x10 => Some(Action::Up),
            0x11 => Some(Action::Down),
            0x12 => Some(Action::Left),
            0x13 => Some(Action::Right),
            0x14 => Some(Action::Select),
            0x15 => Some(Action::Back),
            0x16 => Some(Action::Menu),
            0x17 => Some(Action::Home),
            0x18 => Some(Action::SteerLeft),
            0x19 => Some(Action::SteerRight),
            0x20 => Some(Action::Emote),
            0x30 => Some(Action::IncreaseDifficulty),
            0x31 => Some(Action::DecreaseDifficulty),
            0x32 => Some(Action::SkipInterval),
            0x33 => Some(Action::Pause),
            0x34 => Some(Action::Resume),
            0x37 => Some(Action::UTurn),
            0x40 => Some(Action::CameraView),
            0x44 => Some(Action::HudToggle),
            0x45 => Some(Action::MapToggle),
            0x50 => Some(Action::PowerUp),
            _ => None,
        }
    }

    pub fn swapped(self) -> Action {
        match self {
            Action::ShiftUp => Action::ShiftDown,
            Action::ShiftDown => Action::ShiftUp,
            other => other,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum EngineEvent {
    Log {
        message: String,
    },
    DeviceConnected {
        id: String,
        name: String,
        kind: String,
    },
    DeviceDisconnected {
        id: String,
        name: String,
    },
    Battery {
        id: String,
        level: u8,
    },
    Unlocked {
        id: String,
    },
    Button {
        device: String,
        button: Button,
        pressed: bool,
    },
    Action {
        action: Action,
        pressed: bool,
    },
    OutputClient {
        peer: String,
        connected: bool,
    },
    TrainerApp {
        peer: String,
        app: String,
        version: String,
        supported: Vec<Action>,
    },
    OutputReady {
        description: String,
    },
    Notification {
        device: String,
        characteristic: String,
        hex: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_round_trips_through_its_protocol_id() {
        for action in Action::ALL {
            let id = action.obc_id().expect("every listed action has an id");
            assert_eq!(Action::from_obc_id(id), Some(action));
        }
        assert_eq!(Action::None.obc_id(), None);
        assert_eq!(Action::from_obc_id(0x7F), None);
    }

    #[test]
    fn swap_only_touches_gears() {
        assert_eq!(Action::ShiftUp.swapped(), Action::ShiftDown);
        assert_eq!(Action::Menu.swapped(), Action::Menu);
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}
