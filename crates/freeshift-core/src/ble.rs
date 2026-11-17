use std::collections::{HashMap, HashSet};
use std::future::pending;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use btleplug::api::{
    Central, CentralEvent, CharPropFlags, Characteristic, Manager as _, Peripheral as _, PeripheralProperties, ScanFilter,
    WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral, PeripheralId};
use futures::StreamExt;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::time::{Instant, interval, sleep, sleep_until, timeout};
use uuid::Uuid;

use crate::config::{Config, CustomTrigger};
use crate::model::{Button, EngineEvent, hex};
use crate::zwift::{self, Frame, ZwiftKind};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const RETRY_DELAY: Duration = Duration::from_secs(5);
const KEEPALIVE: Duration = Duration::from_secs(3);
const IDLE_RESET: Duration = Duration::from_secs(55);

#[derive(Debug)]
pub enum RawInput {
    State { device: String, pressed: Vec<Button> },
    Gone { device: String },
    Pulse { device: String, button: Button },
}

#[derive(Clone, Debug)]
enum Target {
    Zwift(ZwiftKind),
    Custom(CustomTrigger),
}

fn is_connect_error(text: &str) -> bool {
    ["connection failed", "connection timed out", "service discovery"]
        .iter()
        .any(|marker| text.contains(marker))
}

pub async fn first_adapter() -> Result<Adapter> {
    let manager = Manager::new().await.context("Bluetooth is not available")?;
    let adapters = manager.adapters().await.context("listing Bluetooth adapters")?;
    adapters
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("no Bluetooth adapter found; turn Bluetooth on in Windows settings"))
}

pub fn display_name(properties: &PeripheralProperties) -> String {
    properties
        .local_name
        .clone()
        .or_else(|| properties.advertisement_name.clone())
        .unwrap_or_default()
}

fn classify(properties: &PeripheralProperties, config: &Config) -> Option<Target> {
    let name = display_name(properties).to_lowercase();
    let custom = config.custom_triggers.iter().find(|trigger| {
        let prefix = trigger.name_prefix.trim().to_lowercase();
        !prefix.is_empty() && !trigger.characteristic.trim().is_empty() && name.starts_with(&prefix)
    });
    if let Some(trigger) = custom {
        return Some(Target::Custom(trigger.clone()));
    }
    if !config.zwift_enabled {
        return None;
    }
    if let Some(kind) = ZwiftKind::from_manufacturer_data(&properties.manufacturer_data) {
        return kind.is_supported().then_some(Target::Zwift(kind));
    }
    name.starts_with("zwift").then_some(Target::Zwift(ZwiftKind::Unknown(0)))
}

struct Scanner {
    adapter: Adapter,
    config: Config,
    raw: mpsc::UnboundedSender<RawInput>,
    events: broadcast::Sender<EngineEvent>,
    stop: watch::Receiver<bool>,
    done: mpsc::UnboundedSender<(PeripheralId, bool)>,
    active: HashSet<PeripheralId>,
    failing: HashSet<PeripheralId>,
    attempts: HashMap<PeripheralId, Instant>,
}

impl Scanner {
    async fn consider(&mut self, id: PeripheralId) {
        if self.active.contains(&id) {
            return;
        }
        if self.attempts.get(&id).is_some_and(|at| at.elapsed() < RETRY_DELAY) {
            return;
        }
        let Ok(peripheral) = self.adapter.peripheral(&id).await else {
            return;
        };
        let Ok(Some(properties)) = peripheral.properties().await else {
            return;
        };
        let Some(target) = classify(&properties, &self.config) else {
            return;
        };
        self.active.insert(id.clone());
        self.attempts.insert(id.clone(), Instant::now());
        let name = display_name(&properties);
        let raw = self.raw.clone();
        let events = self.events.clone();
        let stop = self.stop.clone();
        let done = self.done.clone();
        let window = Duration::from_millis(self.config.double_tap_ms);
        let is_repeat_failure = self.failing.contains(&id);
        tokio::spawn(async move {
            let label = if name.is_empty() { peripheral.address().to_string() } else { name.clone() };
            let result = match target {
                Target::Zwift(kind) => zwift_session(&peripheral, kind, &label, &raw, &events, stop).await,
                Target::Custom(trigger) => trigger_session(&peripheral, &trigger, &label, window, &raw, &events, stop).await,
            };
            let mut is_connect_failure = false;
            if let Err(error) = result {
                let text = format!("{error:#}");
                is_connect_failure = is_connect_error(&text);
                if !is_connect_failure {
                    let _ = events.send(EngineEvent::Log { message: format!("{label}: {text}") });
                } else if !is_repeat_failure {
                    let _ = events.send(EngineEvent::Log {
                        message: format!("{label}: can't connect right now ({text}). Press a button on it to wake it; FreeShift keeps trying quietly."),
                    });
                }
            }
            let _ = peripheral.disconnect().await;
            let _ = done.send((id, is_connect_failure));
        });
    }
}

pub async fn run_scanner(
    config: Config,
    raw: mpsc::UnboundedSender<RawInput>,
    events: broadcast::Sender<EngineEvent>,
    mut stop: watch::Receiver<bool>,
) -> Result<()> {
    let adapter = first_adapter().await?;
    let mut central_events = adapter.events().await.context("listening for Bluetooth events")?;
    adapter
        .start_scan(ScanFilter::default())
        .await
        .context("starting Bluetooth scan; is Bluetooth turned on?")?;
    let _ = events.send(EngineEvent::Log { message: "Scanning for controllers".to_string() });

    let (done, mut finished) = mpsc::unbounded_channel();
    let mut scanner = Scanner {
        adapter: adapter.clone(),
        config,
        raw,
        events,
        stop: stop.clone(),
        done,
        active: HashSet::new(),
        failing: HashSet::new(),
        attempts: HashMap::new(),
    };
    let mut sweep = interval(Duration::from_secs(10));

    loop {
        tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    break;
                }
            }
            Some((id, is_connect_failure)) = finished.recv() => {
                scanner.active.remove(&id);
                if is_connect_failure {
                    scanner.failing.insert(id);
                } else {
                    scanner.failing.remove(&id);
                }
            }
            event = central_events.next() => {
                match event {
                    Some(CentralEvent::DeviceDiscovered(id))
                    | Some(CentralEvent::DeviceUpdated(id))
                    | Some(CentralEvent::ManufacturerDataAdvertisement { id, .. }) => scanner.consider(id).await,
                    Some(_) => {}
                    None => break,
                }
            }
            _ = sweep.tick() => {
                if let Ok(peripherals) = adapter.peripherals().await {
                    for peripheral in peripherals {
                        scanner.consider(peripheral.id()).await;
                    }
                }
            }
        }
    }
    let _ = adapter.stop_scan().await;
    Ok(())
}

pub async fn write(peripheral: &Peripheral, characteristic: &Characteristic, bytes: &[u8]) -> Result<()> {
    let kind = if characteristic.properties.contains(CharPropFlags::WRITE_WITHOUT_RESPONSE) {
        WriteType::WithoutResponse
    } else {
        WriteType::WithResponse
    };
    peripheral.write(characteristic, bytes, kind).await.context("write failed")
}

pub async fn connect(peripheral: &Peripheral) -> Result<()> {
    timeout(CONNECT_TIMEOUT, peripheral.connect())
        .await
        .context("connection timed out")?
        .context("connection failed")?;
    timeout(CONNECT_TIMEOUT, peripheral.discover_services())
        .await
        .context("service discovery timed out")?
        .context("service discovery failed")?;
    Ok(())
}

fn find(peripheral: &Peripheral, uuid: Uuid) -> Option<Characteristic> {
    peripheral.characteristics().into_iter().find(|characteristic| characteristic.uuid == uuid)
}

async fn zwift_session(
    peripheral: &Peripheral,
    kind: ZwiftKind,
    label: &str,
    raw: &mpsc::UnboundedSender<RawInput>,
    events: &broadcast::Sender<EngineEvent>,
    mut stop: watch::Receiver<bool>,
) -> Result<()> {
    let device = peripheral.address().to_string();
    connect(peripheral).await?;
    let async_characteristic = find(peripheral, zwift::ASYNC)
        .context("Zwift service not found; update the controller firmware in Zwift Companion")?;
    let sync_rx = find(peripheral, zwift::SYNC_RX).context("Zwift command characteristic not found")?;
    let sync_tx = find(peripheral, zwift::SYNC_TX);

    let mut notifications = peripheral.notifications().await.context("opening notification stream")?;
    peripheral
        .subscribe(&async_characteristic)
        .await
        .context("subscribing to button events")?;
    if let Some(characteristic) = &sync_tx {
        if let Err(error) = peripheral.subscribe(characteristic).await {
            let _ = events.send(EngineEvent::Log { message: format!("{label}: handshake channel unavailable ({error})") });
        }
    }

    if kind.is_click_v1() {
        write(peripheral, &sync_rx, &zwift::RIDE_ON).await?;
    } else {
        for frame in [&zwift::ACTIVATION_START[..], &zwift::ACTIVATION_SECOND[..], &zwift::ACTIVATION_THIRD[..]] {
            write(peripheral, &sync_rx, frame).await?;
            sleep(Duration::from_millis(100)).await;
        }
    }
    let _ = events.send(EngineEvent::DeviceConnected {
        id: device.clone(),
        name: label.to_string(),
        kind: kind.label(),
    });

    let mut keepalive = interval(KEEPALIVE);
    keepalive.tick().await;
    let mut last_activity = Instant::now();
    let mut reset_sent = false;
    let mut unlocked = false;

    let outcome = loop {
        tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    break Ok(());
                }
            }
            notification = notifications.next() => {
                let Some(notification) = notification else {
                    break Err(anyhow!("connection lost"));
                };
                last_activity = Instant::now();
                if notification.uuid == zwift::ASYNC {
                    match zwift::parse_async(&notification.value) {
                        Frame::Pressed(pressed) => {
                            let _ = raw.send(RawInput::State { device: device.clone(), pressed });
                        }
                        Frame::Battery(level) => {
                            let _ = events.send(EngineEvent::Battery { id: device.clone(), level });
                        }
                        Frame::Challenge => {
                            let _ = write(peripheral, &sync_rx, &zwift::UNLOCK_ACK).await;
                        }
                        Frame::Disconnect => break Err(anyhow!("another app took over the controller")),
                        Frame::Idle | Frame::Other => {}
                    }
                } else if notification.uuid == zwift::SYNC_TX && !unlocked && zwift::is_unlock_echo(&notification.value) {
                    unlocked = true;
                    let _ = events.send(EngineEvent::Unlocked { id: device.clone() });
                    let _ = write(peripheral, &sync_rx, &zwift::UNLOCK_ACK).await;
                }
            }
            _ = keepalive.tick() => {
                if kind.is_click_v1() {
                    if !peripheral.is_connected().await.unwrap_or(false) {
                        break Err(anyhow!("connection lost"));
                    }
                    continue;
                }
                if !reset_sent && last_activity.elapsed() >= IDLE_RESET {
                    reset_sent = true;
                    if write(peripheral, &sync_rx, &zwift::POD_RESET).await.is_err() {
                        break Err(anyhow!("connection lost"));
                    }
                    continue;
                }
                if write(peripheral, &sync_rx, &zwift::RIDE_ON).await.is_err() {
                    break Err(anyhow!("connection lost"));
                }
            }
        }
    };

    let _ = raw.send(RawInput::Gone { device: device.clone() });
    let _ = events.send(EngineEvent::DeviceDisconnected { id: device, name: label.to_string() });
    outcome
}

async fn trigger_session(
    peripheral: &Peripheral,
    trigger: &CustomTrigger,
    label: &str,
    window: Duration,
    raw: &mpsc::UnboundedSender<RawInput>,
    events: &broadcast::Sender<EngineEvent>,
    mut stop: watch::Receiver<bool>,
) -> Result<()> {
    let device = peripheral.address().to_string();
    let uuid = Uuid::parse_str(trigger.characteristic.trim()).context("trigger characteristic is not a valid UUID")?;
    connect(peripheral).await?;
    let characteristic = find(peripheral, uuid).with_context(|| format!("characteristic {uuid} not found"))?;
    let mut notifications = peripheral.notifications().await.context("opening notification stream")?;
    peripheral.subscribe(&characteristic).await.context("subscribing to trigger")?;
    let _ = events.send(EngineEvent::DeviceConnected {
        id: device.clone(),
        name: label.to_string(),
        kind: "Custom trigger (tap / double tap)".to_string(),
    });

    let mut health = interval(KEEPALIVE);
    let mut taps = 0u32;
    let mut deadline: Option<Instant> = None;

    let outcome = loop {
        let wait = async {
            match deadline {
                Some(at) => sleep_until(at).await,
                None => pending::<()>().await,
            }
        };
        tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    break Ok(());
                }
            }
            notification = notifications.next() => {
                let Some(notification) = notification else {
                    break Err(anyhow!("connection lost"));
                };
                if notification.uuid != uuid {
                    continue;
                }
                let _ = events.send(EngineEvent::Notification {
                    device: label.to_string(),
                    characteristic: uuid.to_string(),
                    hex: hex(&notification.value),
                });
                taps += 1;
                if taps >= 2 {
                    taps = 0;
                    deadline = None;
                    let _ = raw.send(RawInput::Pulse { device: device.clone(), button: Button::DoubleTap });
                } else {
                    deadline = Some(Instant::now() + window);
                }
            }
            _ = wait => {
                taps = 0;
                deadline = None;
                let _ = raw.send(RawInput::Pulse { device: device.clone(), button: Button::Tap });
            }
            _ = health.tick() => {
                if !peripheral.is_connected().await.unwrap_or(false) {
                    break Err(anyhow!("connection lost"));
                }
            }
        }
    };

    let _ = events.send(EngineEvent::DeviceDisconnected { id: device, name: label.to_string() });
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_connect_errors() {
        assert!(is_connect_error("connection failed: Not connected"));
        assert!(is_connect_error("service discovery timed out: deadline has elapsed"));
        assert!(!is_connect_error("connection lost"));
        assert!(!is_connect_error("another app took over the controller"));
    }
}
