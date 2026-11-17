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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Action {
    ShiftUp,
    ShiftDown,
    #[default]
    None,
}

impl Action {
    pub fn swapped(self) -> Action {
        match self {
            Action::ShiftUp => Action::ShiftDown,
            Action::ShiftDown => Action::ShiftUp,
            Action::None => Action::None,
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
    OutputReady {
        description: String,
    },
    Notification {
        device: String,
        characteristic: String,
        hex: String,
    },
}

pub fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}
