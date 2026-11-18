use std::collections::{HashMap, HashSet};
use std::future::pending;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use btleplug::api::{
    Central, CentralEvent, CentralState, CharPropFlags, Characteristic, Manager as _, Peripheral as _,
    PeripheralProperties, ScanFilter, WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral, PeripheralId};
use futures::{Stream, StreamExt};
use tokio::sync::{Mutex, broadcast, mpsc, watch};
use tokio::time::{Instant, interval, sleep, sleep_until, timeout};
use uuid::Uuid;

use crate::config::{Config, CustomTrigger};
use crate::link::Link;
use crate::model::{Button, EngineEvent, hex};
use crate::zwift::{self, Frame, ZwiftKind};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const RETRY_DELAY: Duration = Duration::from_secs(5);
const KEEPALIVE: Duration = Duration::from_secs(3);
const IDLE_RESET: Duration = Duration::from_secs(55);
const POST_RESET_GRACE: Duration = Duration::from_secs(15);
const SEEN_WINDOW: Duration = Duration::from_secs(30);
const DISCOVERY_ATTEMPTS: u32 = 4;
const DISCOVERY_RETRY: Duration = Duration::from_millis(500);
const SWEEP: Duration = Duration::from_secs(10);
const QUIET_RESET: Duration = Duration::from_secs(30);
const RESET_PAUSE: Duration = Duration::from_secs(1);
const REOPEN_DELAY: Duration = Duration::from_secs(5);

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

fn failure_key(text: &str) -> String {
    if is_connect_error(text) { "connect".to_string() } else { text.to_string() }
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
    done: mpsc::UnboundedSender<(PeripheralId, Option<String>)>,
    active: HashSet<PeripheralId>,
    failing: HashMap<PeripheralId, String>,
    attempts: HashMap<PeripheralId, Instant>,
    seen: HashMap<PeripheralId, Instant>,
    connecting: Arc<Mutex<()>>,
}

struct Access {
    adapter: Adapter,
    connecting: Arc<Mutex<()>>,
    stop: watch::Receiver<bool>,
}

fn is_recent(seen: &HashMap<PeripheralId, Instant>, id: &PeripheralId) -> bool {
    seen.get(id).is_some_and(|at| at.elapsed() < SEEN_WINDOW)
}

impl Scanner {
    fn mark_seen(&mut self, id: &PeripheralId) {
        self.seen.insert(id.clone(), Instant::now());
    }

    fn awake(&mut self) -> Vec<PeripheralId> {
        self.seen.retain(|_, at| at.elapsed() < SEEN_WINDOW);
        self.seen.keys().cloned().collect()
    }

    async fn consider(&mut self, id: PeripheralId) {
        if self.active.contains(&id) || !is_recent(&self.seen, &id) {
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
        let previous_failure = self.failing.get(&id).cloned();
        let access = Access {
            adapter: self.adapter.clone(),
            connecting: self.connecting.clone(),
            stop: self.stop.clone(),
        };
        tokio::spawn(async move {
            let label = if name.is_empty() { peripheral.address().to_string() } else { name.clone() };
            let result = match target {
                Target::Zwift(kind) => zwift_session(&peripheral, kind, &label, &access, &raw, &events, stop).await,
                Target::Custom(trigger) => {
                    trigger_session(&peripheral, &trigger, &label, window, &access, &raw, &events, stop).await
                }
            };
            let failure = result.err().map(|error| format!("{error:#}"));
            if let Some(text) = &failure {
                let key = failure_key(text);
                if previous_failure.as_deref() != Some(key.as_str()) {
                    let message = if is_connect_error(text) {
                        format!("{label}: can't connect right now ({text}). Press a button on it to wake it; FreeShift keeps trying quietly.")
                    } else {
                        format!("{label}: {text}")
                    };
                    let _ = events.send(EngineEvent::Log { message });
                }
            }
            let _ = peripheral.disconnect().await;
            let _ = done.send((id, failure.as_deref().map(failure_key)));
        });
    }
}

type EventStream = Pin<Box<dyn Stream<Item = CentralEvent> + Send>>;

enum Pass {
    Stopped,
    Reset { reason: &'static str, announce: bool },
}

async fn wait_or_stop(stop: &mut watch::Receiver<bool>, delay: Duration) -> bool {
    tokio::select! {
        _ = sleep(delay) => *stop.borrow(),
        changed = stop.changed() => changed.is_err() || *stop.borrow(),
    }
}

async fn open_scan() -> Result<(Adapter, EventStream)> {
    let adapter = first_adapter().await?;
    let central_events = adapter.events().await.context("listening for Bluetooth events")?;
    adapter
        .start_scan(ScanFilter::default())
        .await
        .context("starting Bluetooth scan; is Bluetooth turned on?")?;
    Ok((adapter, central_events))
}

async fn open_until_ready(
    events: &broadcast::Sender<EngineEvent>,
    stop: &mut watch::Receiver<bool>,
) -> Option<(Adapter, EventStream)> {
    let mut reported = false;
    loop {
        match open_scan().await {
            Ok(opened) => return Some(opened),
            Err(error) => {
                if !reported {
                    reported = true;
                    let _ = events.send(EngineEvent::Log {
                        message: format!("Bluetooth: {error:#}. FreeShift keeps trying."),
                    });
                }
                if wait_or_stop(stop, REOPEN_DELAY).await {
                    return None;
                }
            }
        }
    }
}

async fn scan_pass(
    scanner: &mut Scanner,
    central_events: &mut EventStream,
    finished: &mut mpsc::UnboundedReceiver<(PeripheralId, Option<String>)>,
    stop: &mut watch::Receiver<bool>,
) -> Pass {
    let mut sweep = interval(SWEEP);
    let mut last_heard = Instant::now();
    let mut heard = false;
    loop {
        tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    return Pass::Stopped;
                }
            }
            Some((id, failure)) = finished.recv() => {
                scanner.active.remove(&id);
                match failure {
                    Some(key) => {
                        scanner.failing.insert(id, key);
                    }
                    None => {
                        scanner.failing.remove(&id);
                    }
                }
            }
            event = central_events.next() => {
                match event {
                    Some(CentralEvent::DeviceDiscovered(id))
                    | Some(CentralEvent::DeviceUpdated(id))
                    | Some(CentralEvent::ManufacturerDataAdvertisement { id, .. }) => {
                        last_heard = Instant::now();
                        heard = true;
                        scanner.mark_seen(&id);
                        scanner.consider(id).await;
                    }
                    Some(CentralEvent::StateUpdate(CentralState::PoweredOff)) => {
                        let _ = scanner.events.send(EngineEvent::Log {
                            message: "Bluetooth is off; FreeShift carries on when it is back".to_string(),
                        });
                    }
                    Some(CentralEvent::StateUpdate(CentralState::PoweredOn)) => {
                        return Pass::Reset { reason: "turned back on", announce: true };
                    }
                    None => {
                        return Pass::Reset { reason: "stopped reporting", announce: true };
                    }
                    Some(_) => {}
                }
            }
            _ = sweep.tick() => {
                if last_heard.elapsed() >= QUIET_RESET && scanner.connecting.try_lock().is_ok() {
                    return Pass::Reset { reason: "went quiet", announce: heard };
                }
                for id in scanner.awake() {
                    scanner.consider(id).await;
                }
            }
        }
    }
}

pub async fn run_scanner(
    config: Config,
    raw: mpsc::UnboundedSender<RawInput>,
    events: broadcast::Sender<EngineEvent>,
    mut stop: watch::Receiver<bool>,
) -> Result<()> {
    let Some((adapter, mut central_events)) = open_until_ready(&events, &mut stop).await else {
        return Ok(());
    };
    let _ = events.send(EngineEvent::Log { message: "Scanning for controllers".to_string() });

    let (done, mut finished) = mpsc::unbounded_channel();
    let mut scanner = Scanner {
        adapter,
        config,
        raw,
        events: events.clone(),
        stop: stop.clone(),
        done,
        active: HashSet::new(),
        failing: HashMap::new(),
        attempts: HashMap::new(),
        seen: HashMap::new(),
        connecting: Arc::new(Mutex::new(())),
    };

    loop {
        let pass = scan_pass(&mut scanner, &mut central_events, &mut finished, &mut stop).await;
        let _ = scanner.adapter.stop_scan().await;
        let Pass::Reset { reason, announce } = pass else {
            return Ok(());
        };
        if wait_or_stop(&mut stop, RESET_PAUSE).await {
            return Ok(());
        }
        let Some((adapter, stream)) = open_until_ready(&events, &mut stop).await else {
            return Ok(());
        };
        scanner.adapter = adapter;
        scanner.seen.clear();
        central_events = stream;
        if announce {
            let _ = events.send(EngineEvent::Log { message: format!("Bluetooth {reason}; scanning for controllers again") });
        }
    }
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

async fn connect_for(
    peripheral: &Peripheral,
    access: &Access,
    required: Uuid,
) -> Result<(Link, Option<Characteristic>)> {
    let _turn = access.connecting.lock().await;
    let _ = access.adapter.stop_scan().await;
    let result = open_and_discover(peripheral, required).await;
    if !*access.stop.borrow() {
        let _ = access.adapter.start_scan(ScanFilter::default()).await;
    }
    result
}

async fn open_and_discover(peripheral: &Peripheral, required: Uuid) -> Result<(Link, Option<Characteristic>)> {
    let link = Link::open(peripheral.address(), CONNECT_TIMEOUT)
        .await
        .context("connection failed")?;
    connect(peripheral).await?;
    for attempt in 1..=DISCOVERY_ATTEMPTS {
        if let Some(characteristic) = find(peripheral, required) {
            return Ok((link, Some(characteristic)));
        }
        if attempt == DISCOVERY_ATTEMPTS {
            break;
        }
        sleep(DISCOVERY_RETRY).await;
        timeout(CONNECT_TIMEOUT, peripheral.discover_services())
            .await
            .context("service discovery timed out")?
            .context("service discovery failed")?;
    }
    Ok((link, None))
}

fn post_reset_silent(reset_at: Option<Instant>, last_activity: Instant) -> bool {
    reset_at.is_some_and(|at| at.elapsed() >= POST_RESET_GRACE && last_activity < at)
}

async fn zwift_session(
    peripheral: &Peripheral,
    kind: ZwiftKind,
    label: &str,
    access: &Access,
    raw: &mpsc::UnboundedSender<RawInput>,
    events: &broadcast::Sender<EngineEvent>,
    mut stop: watch::Receiver<bool>,
) -> Result<()> {
    let device = peripheral.address().to_string();
    let missing_service = match kind {
        ZwiftKind::ClickV2Left => {
            "Zwift service hidden: this left Click v2 pod is locked to Zwift. The right pod works without it"
        }
        _ => "Zwift service not found; the controller may be locked to Zwift or need a firmware update in Zwift Companion",
    };
    let (_link, async_characteristic) = connect_for(peripheral, access, zwift::ASYNC).await?;
    let async_characteristic = async_characteristic.context(missing_service)?;
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
    let mut reset_at: Option<Instant> = None;
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
                if reset_at.is_none() && last_activity.elapsed() >= IDLE_RESET {
                    reset_at = Some(Instant::now());
                    if write(peripheral, &sync_rx, &zwift::POD_RESET).await.is_err() {
                        break Err(anyhow!("connection lost"));
                    }
                    continue;
                }
                if post_reset_silent(reset_at, last_activity) {
                    break Ok(());
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
    access: &Access,
    raw: &mpsc::UnboundedSender<RawInput>,
    events: &broadcast::Sender<EngineEvent>,
    mut stop: watch::Receiver<bool>,
) -> Result<()> {
    let device = peripheral.address().to_string();
    let uuid = Uuid::parse_str(trigger.characteristic.trim()).context("trigger characteristic is not a valid UUID")?;
    let (_link, characteristic) = connect_for(peripheral, access, uuid).await?;
    let characteristic = characteristic.with_context(|| format!("characteristic {uuid} not found"))?;
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

    #[test]
    fn groups_connect_errors_under_one_key() {
        assert_eq!(failure_key("connection failed: Not connected"), "connect");
        assert_eq!(failure_key("connection timed out: deadline has elapsed"), "connect");
        assert_eq!(failure_key("connection lost"), "connection lost");
    }

    #[test]
    fn recycles_a_session_only_when_silent_after_its_reset() {
        let now = Instant::now();
        let long_ago = now - Duration::from_secs(60);
        let reset = now - POST_RESET_GRACE - Duration::from_secs(1);
        assert!(!post_reset_silent(None, long_ago));
        assert!(post_reset_silent(Some(reset), long_ago));
        assert!(!post_reset_silent(Some(reset), now));
        assert!(!post_reset_silent(Some(now), long_ago));
    }
}
