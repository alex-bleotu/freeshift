use std::collections::HashMap;

use uuid::Uuid;

use crate::model::Button;
use crate::protobuf::{field, varint_fields};

pub const COMPANY_ID: u16 = 0x094A;
pub const SERVICE: Uuid = Uuid::from_u128(0x00000001_19ca_4651_86e5_fa29dcdd09d1);
pub const RIDE_SERVICE: Uuid = Uuid::from_u128(0x0000fc82_0000_1000_8000_00805f9b34fb);
pub const ASYNC: Uuid = Uuid::from_u128(0x00000002_19ca_4651_86e5_fa29dcdd09d1);
pub const SYNC_RX: Uuid = Uuid::from_u128(0x00000003_19ca_4651_86e5_fa29dcdd09d1);
pub const SYNC_TX: Uuid = Uuid::from_u128(0x00000004_19ca_4651_86e5_fa29dcdd09d1);

pub const RIDE_ON: [u8; 6] = *b"RideOn";
pub const ACTIVATION_START: [u8; 8] = [b'R', b'i', b'd', b'e', b'O', b'n', 0x02, 0x03];
pub const ACTIVATION_SECOND: [u8; 3] = [0x00, 0x08, 0x00];
pub const ACTIVATION_THIRD: [u8; 3] = [0x00, 0x08, 0x10];
pub const UNLOCK_ACK: [u8; 3] = [0xFF, 0x04, 0x00];
pub const POD_RESET: [u8; 1] = [0x18];

pub const CLICK_V1_MESSAGE: u8 = 0x37;
pub const KEYPAD_MESSAGE: u8 = 0x23;
pub const IDLE_MESSAGE: u8 = 0x15;
pub const BATTERY_MESSAGE: u8 = 0x19;
pub const DISCONNECT_MESSAGE: u8 = 0xFE;

const KEYPAD_BITS: [(u32, Button); 10] = [
    (0x0001, Button::Left),
    (0x0002, Button::Up),
    (0x0004, Button::Right),
    (0x0008, Button::Down),
    (0x0010, Button::A),
    (0x0020, Button::B),
    (0x0040, Button::Y),
    (0x0080, Button::Z),
    (0x0100, Button::Minus),
    (0x1000, Button::Plus),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZwiftKind {
    ClickV1,
    ClickV2Right,
    ClickV2Left,
    PlayRight,
    PlayLeft,
    RideRight,
    RideLeft,
    Unknown(u8),
}

impl ZwiftKind {
    pub fn from_type_byte(byte: u8) -> ZwiftKind {
        match byte {
            0x09 => ZwiftKind::ClickV1,
            0x0A => ZwiftKind::ClickV2Right,
            0x0B => ZwiftKind::ClickV2Left,
            0x02 => ZwiftKind::PlayRight,
            0x03 => ZwiftKind::PlayLeft,
            0x07 => ZwiftKind::RideRight,
            0x08 => ZwiftKind::RideLeft,
            other => ZwiftKind::Unknown(other),
        }
    }

    pub fn from_manufacturer_data(data: &HashMap<u16, Vec<u8>>) -> Option<ZwiftKind> {
        data.get(&COMPANY_ID)
            .and_then(|bytes| bytes.first())
            .map(|byte| ZwiftKind::from_type_byte(*byte))
    }

    pub fn is_click_v1(self) -> bool {
        self == ZwiftKind::ClickV1
    }

    pub fn is_supported(self) -> bool {
        !matches!(self, ZwiftKind::PlayRight | ZwiftKind::PlayLeft)
    }

    pub fn label(self) -> String {
        match self {
            ZwiftKind::ClickV1 => "Zwift Click".to_string(),
            ZwiftKind::ClickV2Right => "Zwift Click v2 (right)".to_string(),
            ZwiftKind::ClickV2Left => "Zwift Click v2 (left)".to_string(),
            ZwiftKind::PlayRight => "Zwift Play (right)".to_string(),
            ZwiftKind::PlayLeft => "Zwift Play (left)".to_string(),
            ZwiftKind::RideRight => "Zwift Ride (right)".to_string(),
            ZwiftKind::RideLeft => "Zwift Ride (left)".to_string(),
            ZwiftKind::Unknown(byte) => format!("Zwift device 0x{byte:02X}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Frame {
    Pressed(Vec<Button>),
    Battery(u8),
    Challenge,
    Idle,
    Disconnect,
    Other,
}

pub fn parse_async(bytes: &[u8]) -> Frame {
    let Some((&kind, payload)) = bytes.split_first() else {
        return Frame::Other;
    };
    match kind {
        CLICK_V1_MESSAGE => parse_click_v1(payload).map_or(Frame::Other, Frame::Pressed),
        KEYPAD_MESSAGE => parse_keypad(payload).map_or(Frame::Other, Frame::Pressed),
        BATTERY_MESSAGE => varint_fields(payload)
            .and_then(|fields| field(&fields, 1))
            .and_then(|level| u8::try_from(level).ok())
            .map_or(Frame::Other, Frame::Battery),
        IDLE_MESSAGE => Frame::Idle,
        DISCONNECT_MESSAGE => Frame::Disconnect,
        0xFF if payload.first() == Some(&0x03) => Frame::Challenge,
        _ => Frame::Other,
    }
}

fn parse_click_v1(payload: &[u8]) -> Option<Vec<Button>> {
    let fields = varint_fields(payload)?;
    let mut pressed = Vec::new();
    if field(&fields, 1).unwrap_or(0) == 0 {
        pressed.push(Button::Plus);
    }
    if field(&fields, 2).unwrap_or(0) == 0 {
        pressed.push(Button::Minus);
    }
    Some(pressed)
}

fn parse_keypad(payload: &[u8]) -> Option<Vec<Button>> {
    let fields = varint_fields(payload)?;
    let bitmap = field(&fields, 1)?;
    Some(
        KEYPAD_BITS
            .iter()
            .filter(|(bit, _)| bitmap & u64::from(*bit) == 0)
            .map(|(_, button)| *button)
            .collect(),
    )
}

pub fn is_unlock_echo(bytes: &[u8]) -> bool {
    bytes.first() == Some(&b'R')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_v1_plus_pressed() {
        assert_eq!(parse_async(&[0x37, 0x08, 0x00, 0x10, 0x01]), Frame::Pressed(vec![Button::Plus]));
    }

    #[test]
    fn click_v1_minus_pressed() {
        assert_eq!(parse_async(&[0x37, 0x08, 0x01, 0x10, 0x00]), Frame::Pressed(vec![Button::Minus]));
    }

    #[test]
    fn click_v1_nothing_pressed() {
        assert_eq!(parse_async(&[0x37, 0x08, 0x01, 0x10, 0x01]), Frame::Pressed(vec![]));
    }

    #[test]
    fn click_v1_omitted_field_means_pressed() {
        assert_eq!(parse_async(&[0x37, 0x10, 0x01]), Frame::Pressed(vec![Button::Plus]));
    }

    #[test]
    fn keypad_all_released() {
        let frame = [0x23, 0x08, 0xFF, 0xFF, 0x03];
        assert_eq!(parse_async(&frame), Frame::Pressed(vec![]));
    }

    #[test]
    fn keypad_plus_pressed() {
        let bitmap: u64 = 0xFFFF & !0x1000;
        let mut frame = vec![0x23, 0x08];
        frame.extend(encode_varint(bitmap));
        assert_eq!(parse_async(&frame), Frame::Pressed(vec![Button::Plus]));
    }

    #[test]
    fn keypad_minus_and_b_pressed() {
        let bitmap: u64 = 0xFFFF & !0x0100 & !0x0020;
        let mut frame = vec![0x23, 0x08];
        frame.extend(encode_varint(bitmap));
        assert_eq!(parse_async(&frame), Frame::Pressed(vec![Button::B, Button::Minus]));
    }

    #[test]
    fn battery_level() {
        assert_eq!(parse_async(&[0x19, 0x08, 0x5A]), Frame::Battery(90));
    }

    #[test]
    fn challenge_and_idle() {
        assert_eq!(parse_async(&[0xFF, 0x03, 0x01, 0x02]), Frame::Challenge);
        assert_eq!(parse_async(&[0xFF, 0x05, 0x00]), Frame::Other);
        assert_eq!(parse_async(&[0x15]), Frame::Idle);
        assert_eq!(parse_async(&[]), Frame::Other);
    }

    #[test]
    fn kind_from_advertisement() {
        let mut data = HashMap::new();
        data.insert(COMPANY_ID, vec![0x0A, 0x01]);
        assert_eq!(ZwiftKind::from_manufacturer_data(&data), Some(ZwiftKind::ClickV2Right));
        data.insert(COMPANY_ID, vec![0x09]);
        assert_eq!(ZwiftKind::from_manufacturer_data(&data), Some(ZwiftKind::ClickV1));
        assert_eq!(ZwiftKind::from_manufacturer_data(&HashMap::new()), None);
    }

    fn encode_varint(mut value: u64) -> Vec<u8> {
        let mut bytes = Vec::new();
        loop {
            let byte = (value & 0x7F) as u8;
            value >>= 7;
            if value == 0 {
                bytes.push(byte);
                return bytes;
            }
            bytes.push(byte | 0x80);
        }
    }
}
