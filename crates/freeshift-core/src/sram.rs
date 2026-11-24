use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use aes::Aes128;
use anyhow::{Context, Result, anyhow};
use btleplug::api::PeripheralProperties;
use eax::Eax;
use eax::aead::generic_array::GenericArray;
use eax::aead::{Aead, KeyInit};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::protobuf::{field, varint_fields};

pub const ADVERTISED_SERVICE: Uuid = Uuid::from_u128(0x0000fe51_0000_1000_8000_00805f9b34fb);
pub const BOND: Uuid = Uuid::from_u128(0xd905ee52_90aa_4c7c_b036_1e01fb8eb7ee);
pub const TRIGGER: Uuid = Uuid::from_u128(0xd9050054_90aa_4c7c_b036_1e01fb8eb7ee);
pub const BATTERY_LEVEL: Uuid = Uuid::from_u128(0x00002a19_0000_1000_8000_00805f9b34fb);

pub const BOND_INIT: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
pub const BOND_FINISH: [u8; 1] = [0x73];
pub const PRESS_EDGE: u8 = 0xFF;
pub const KEY_TRANSPORT_LEN: usize = 48;

const MODULUS: u128 = u128::MAX - 712;
const GENERATOR: u128 = 5;
const NONCE_LEN: usize = 16;
const TAG_LEN: usize = 16;
const REAR_DERAILLEUR: u8 = 0x81;
const DEVICE_TYPE_FIELD: u32 = 2;
const BUTTON_MASK_FIELD: u32 = 3;

type SramEax = Eax<Aes128>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Press {
    pub device_type: u64,
    pub button_mask: u64,
}

impl Press {
    pub fn side(self) -> Option<Side> {
        match self.device_type {
            0 => Some(Side::Left),
            1 => Some(Side::Right),
            _ => None,
        }
    }
}

pub fn is_rear_derailleur(properties: &PeripheralProperties) -> bool {
    if let Some(data) = properties.service_data.get(&ADVERTISED_SERVICE) {
        return data.last() == Some(&REAR_DERAILLEUR);
    }
    let name = properties.local_name.clone().or_else(|| properties.advertisement_name.clone()).unwrap_or_default();
    properties.services.contains(&ADVERTISED_SERVICE) && name.starts_with("SRAM")
}

fn add_mod(left: u128, right: u128) -> u128 {
    let (sum, overflowed) = left.overflowing_add(right);
    if overflowed || sum >= MODULUS { sum.wrapping_sub(MODULUS) } else { sum }
}

fn mul_mod(left: u128, right: u128) -> u128 {
    let mut result = 0;
    let mut addend = left % MODULUS;
    let mut multiplier = right;
    while multiplier > 0 {
        if multiplier & 1 == 1 {
            result = add_mod(result, addend);
        }
        addend = add_mod(addend, addend);
        multiplier >>= 1;
    }
    result
}

fn pow_mod(base: u128, exponent: u128) -> u128 {
    let mut result = 1;
    let mut square = base % MODULUS;
    let mut remaining = exponent;
    while remaining > 0 {
        if remaining & 1 == 1 {
            result = mul_mod(result, square);
        }
        square = mul_mod(square, square);
        remaining >>= 1;
    }
    result
}

pub struct Handshake {
    private: u128,
}

impl Handshake {
    pub fn new() -> Result<Handshake> {
        loop {
            let mut bytes = [0u8; 16];
            getrandom::fill(&mut bytes).map_err(|error| anyhow!("no random numbers available: {error}"))?;
            let private = u128::from_le_bytes(bytes) % MODULUS;
            if private > 1 {
                return Ok(Handshake { private });
            }
        }
    }

    #[cfg(test)]
    fn from_private(private: u128) -> Handshake {
        Handshake { private }
    }

    pub fn public_key(&self) -> [u8; 16] {
        pow_mod(GENERATOR, self.private).to_be_bytes()
    }

    pub fn shared_secret(&self, device_public: &[u8]) -> Result<[u8; 16]> {
        let bytes: [u8; 16] = device_public.try_into().context("the derailleur's public key is not 16 bytes")?;
        let value = u128::from_be_bytes(bytes);
        if value <= 1 || value >= MODULUS {
            return Err(anyhow!("the derailleur sent an invalid public key"));
        }
        Ok(pow_mod(value, self.private).to_be_bytes())
    }

    pub fn device_key(&self, device_public: &[u8], transport: &[u8]) -> Result<[u8; 16]> {
        let shared = self.shared_secret(device_public)?;
        let plain = open(&shared, transport).context("could not read the key the derailleur sent")?;
        plain.as_slice().try_into().context("the derailleur's key is not 16 bytes")
    }
}

pub fn open(key: &[u8; 16], frame: &[u8]) -> Result<Vec<u8>> {
    if frame.len() < NONCE_LEN + TAG_LEN {
        return Err(anyhow!("frame too short ({} bytes)", frame.len()));
    }
    let cipher = SramEax::new(GenericArray::from_slice(key));
    let (nonce, sealed) = frame.split_at(NONCE_LEN);
    cipher
        .decrypt(GenericArray::from_slice(nonce), sealed)
        .map_err(|_| anyhow!("frame does not match the key"))
}

pub fn decode_press(plain: &[u8]) -> Option<Press> {
    let fields = varint_fields(plain)?;
    Some(Press {
        device_type: field(&fields, DEVICE_TYPE_FIELD)?,
        button_mask: field(&fields, BUTTON_MASK_FIELD).unwrap_or(0),
    })
}

pub fn parse_key(text: &str) -> Option<[u8; 16]> {
    let text = text.trim();
    if text.len() != 32 {
        return None;
    }
    let mut key = [0u8; 16];
    for (index, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(text.get(index * 2..index * 2 + 2)?, 16).ok()?;
    }
    Some(key)
}

pub fn key_text(key: &[u8; 16]) -> String {
    key.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct KeyStore {
    pub devices: BTreeMap<String, String>,
    pub rejected: Vec<String>,
}

pub enum KeySource {
    FreeShift,
    BikeControl,
}

impl KeyStore {
    pub fn path() -> Result<PathBuf> {
        let base = dirs::config_dir().context("no config directory on this system")?;
        Ok(base.join("FreeShift").join("sram_keys.json"))
    }

    pub fn load() -> KeyStore {
        KeyStore::path()
            .ok()
            .and_then(|path| fs::read_to_string(path).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        let path = KeyStore::path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        fs::write(&path, serde_json::to_string_pretty(self)?).with_context(|| format!("writing {}", path.display()))
    }

    pub fn find(&self, address: &str) -> Option<([u8; 16], KeySource)> {
        let address = address.to_uppercase();
        if let Some(key) = self.devices.get(&address).and_then(|text| parse_key(text)) {
            return Some((key, KeySource::FreeShift));
        }
        let key = bikecontrol_key(&address)?;
        if self.rejected.contains(&key_text(&key)) {
            return None;
        }
        Some((key, KeySource::BikeControl))
    }

    pub fn remember(&mut self, address: &str, key: &[u8; 16]) {
        let text = key_text(key);
        self.rejected.retain(|rejected| rejected != &text);
        self.devices.insert(address.to_uppercase(), text);
    }

    pub fn reject(&mut self, address: &str, key: &[u8; 16]) {
        self.devices.remove(&address.to_uppercase());
        let text = key_text(key);
        if !self.rejected.contains(&text) {
            self.rejected.push(text);
        }
    }
}

fn bikecontrol_key(address: &str) -> Option<[u8; 16]> {
    let path = dirs::config_dir()?.join("de.jonasbark").join("BikeControl").join("shared_preferences.json");
    let text = fs::read_to_string(path).ok()?;
    let preferences: serde_json::Value = serde_json::from_str(&text).ok()?;
    let name = format!("flutter.sram_key_{}", address.to_lowercase());
    preferences.get(&name)?.as_str().and_then(parse_key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(text: &str) -> Vec<u8> {
        (0..text.len()).step_by(2).map(|index| u8::from_str_radix(&text[index..index + 2], 16).unwrap()).collect()
    }

    #[test]
    fn modulus_is_two_to_the_128_minus_713() {
        assert_eq!(MODULUS.wrapping_add(713), 0);
    }

    #[test]
    fn modular_arithmetic_wraps_correctly() {
        assert_eq!(add_mod(MODULUS - 1, 2), 1);
        assert_eq!(mul_mod(MODULUS - 1, MODULUS - 1), 1);
        assert_eq!(pow_mod(GENERATOR, MODULUS - 1), 1);
    }

    #[test]
    fn reproduces_the_published_bond_example() {
        let private = u128::from_le_bytes(bytes("6418b20cb4e1d4cf4af19b184aff1d2a").try_into().unwrap());
        let handshake = Handshake::from_private(private);
        assert_eq!(handshake.public_key().to_vec(), bytes("297ca1db5827261af813875fd09800b0"));
        let device_public = bytes("9ac11ad0a4f6c2b99c5559e2d210c410");
        assert_eq!(handshake.shared_secret(&device_public).unwrap().to_vec(), bytes("55406a336a328156b81019d7ac3d5d24"));
        let transport = bytes(
            "8d7a16ed42128ee445b9864f20324a0e0b9e1982be7a9ad3cf611f453696fa8d2f24f1878c44eb7f77ff5b4a3c616395",
        );
        let key = handshake.device_key(&device_public, &transport).unwrap();
        assert_eq!(key.to_vec(), bytes("b0690781867fde13ac1b9d30bbb4004f"));
    }

    #[test]
    fn rejects_a_frame_under_the_wrong_key() {
        let transport = bytes(
            "8d7a16ed42128ee445b9864f20324a0e0b9e1982be7a9ad3cf611f453696fa8d2f24f1878c44eb7f77ff5b4a3c616395",
        );
        assert!(open(&[7u8; 16], &transport).is_err());
        assert!(open(&[7u8; 16], &[0u8; 20]).is_err());
    }

    #[test]
    fn decodes_left_and_right_presses() {
        assert_eq!(decode_press(&[0x10, 0x00, 0x18, 0x01]).unwrap().side(), Some(Side::Left));
        assert_eq!(decode_press(&[0x10, 0x01, 0x18, 0x01]).unwrap().side(), Some(Side::Right));
        assert_eq!(decode_press(&[0x10, 0x04, 0x18, 0x01]).unwrap().side(), None);
        assert_eq!(decode_press(&[0x18, 0x01]), None);
        assert_eq!(decode_press(&[0x10]), None);
    }

    #[test]
    fn key_text_round_trips() {
        let key = parse_key("b0690781867fde13ac1b9d30bbb4004f").unwrap();
        assert_eq!(key_text(&key), "b0690781867fde13ac1b9d30bbb4004f");
        assert!(parse_key("b069").is_none());
        assert!(parse_key("zz690781867fde13ac1b9d30bbb4004f").is_none());
    }
}
