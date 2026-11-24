use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use btleplug::api::{Central, CharPropFlags, Peripheral as _, ScanFilter};
use btleplug::platform::Peripheral;
use futures::StreamExt;
use serde::Serialize;
use tokio::sync::broadcast;
use tokio::time::{Instant, sleep, timeout};

use crate::ble::{connect, display_name, first_adapter};
use crate::link::Link;
use crate::model::{EngineEvent, hex};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeenDevice {
    pub address: String,
    pub name: String,
    pub rssi: Option<i16>,
    pub manufacturer_data: Vec<String>,
    pub service_data: Vec<String>,
    pub services: Vec<String>,
}

pub async fn scan(seconds: u64) -> Result<Vec<SeenDevice>> {
    let adapter = first_adapter().await?;
    adapter.start_scan(ScanFilter::default()).await.context("starting scan")?;
    sleep(Duration::from_secs(seconds)).await;
    let _ = adapter.stop_scan().await;
    let mut devices = Vec::new();
    for peripheral in adapter.peripherals().await? {
        let Ok(Some(properties)) = peripheral.properties().await else {
            continue;
        };
        let mut manufacturer_data: Vec<String> = properties
            .manufacturer_data
            .iter()
            .map(|(company, bytes)| format!("0x{company:04X}: {}", hex(bytes)))
            .collect();
        manufacturer_data.sort();
        let mut service_data: Vec<String> = properties
            .service_data
            .iter()
            .map(|(uuid, bytes)| format!("{uuid}: {}", hex(bytes)))
            .collect();
        service_data.sort();
        devices.push(SeenDevice {
            address: properties.address.to_string(),
            name: display_name(&properties),
            rssi: properties.rssi,
            manufacturer_data,
            service_data,
            services: properties.services.iter().map(ToString::to_string).collect(),
        });
    }
    devices.sort_by(|left, right| right.rssi.cmp(&left.rssi));
    Ok(devices)
}

async fn find_device(filter: &str, wait: Duration) -> Result<Peripheral> {
    let adapter = first_adapter().await?;
    adapter.start_scan(ScanFilter::default()).await.context("starting scan")?;
    let needle = filter.to_lowercase();
    let started = Instant::now();
    while started.elapsed() < wait {
        for peripheral in adapter.peripherals().await? {
            let Ok(Some(properties)) = peripheral.properties().await else {
                continue;
            };
            let name = display_name(&properties).to_lowercase();
            let address = properties.address.to_string().to_lowercase();
            if name.contains(&needle) || address.contains(&needle) {
                let _ = adapter.stop_scan().await;
                return Ok(peripheral);
            }
        }
        sleep(Duration::from_millis(500)).await;
    }
    let _ = adapter.stop_scan().await;
    Err(anyhow!("no device matching \"{filter}\" seen in {} s; wake it up and try again", wait.as_secs()))
}

fn describe(flags: CharPropFlags) -> String {
    let names = [
        (CharPropFlags::READ, "read"),
        (CharPropFlags::WRITE, "write"),
        (CharPropFlags::WRITE_WITHOUT_RESPONSE, "write-no-response"),
        (CharPropFlags::NOTIFY, "notify"),
        (CharPropFlags::INDICATE, "indicate"),
    ];
    names
        .iter()
        .filter(|(flag, _)| flags.contains(*flag))
        .map(|(_, name)| *name)
        .collect::<Vec<_>>()
        .join(", ")
}

pub async fn explore(filter: &str, listen_seconds: u64, events: broadcast::Sender<EngineEvent>) -> Result<()> {
    let log = |message: String| {
        let _ = events.send(EngineEvent::Log { message });
    };
    log(format!("Looking for \"{filter}\" (read-only, nothing is written to the device)"));
    let peripheral = find_device(filter, Duration::from_secs(30)).await?;
    let name = peripheral
        .properties()
        .await
        .ok()
        .flatten()
        .map(|properties| display_name(&properties))
        .unwrap_or_default();
    log(format!("Connecting to {name} {}", peripheral.address()));
    let _link = Link::open(peripheral.address(), Duration::from_secs(15))
        .await
        .context("connection failed")?;
    log("Link is up; reading services".to_string());
    connect(&peripheral).await?;

    for service in peripheral.services() {
        log(format!("service {}", service.uuid));
        for characteristic in &service.characteristics {
            let mut line = format!("  characteristic {} [{}]", characteristic.uuid, describe(characteristic.properties));
            if characteristic.properties.contains(CharPropFlags::READ) {
                if let Ok(Ok(value)) = timeout(Duration::from_secs(3), peripheral.read(characteristic)).await {
                    line.push_str(&format!(" = {}", hex(&value)));
                }
            }
            log(line);
        }
    }

    let mut notifications = peripheral.notifications().await.context("opening notification stream")?;
    let mut subscribed = 0;
    for characteristic in peripheral.characteristics() {
        if characteristic.properties.intersects(CharPropFlags::NOTIFY | CharPropFlags::INDICATE) {
            match peripheral.subscribe(&characteristic).await {
                Ok(()) => subscribed += 1,
                Err(error) => log(format!("  could not subscribe to {}: {error}", characteristic.uuid)),
            }
        }
    }
    log(format!("Listening on {subscribed} characteristics for {listen_seconds} s. Press each button now."));

    let deadline = Instant::now() + Duration::from_secs(listen_seconds);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match timeout(remaining, notifications.next()).await {
            Ok(Some(notification)) => {
                let _ = events.send(EngineEvent::Notification {
                    device: name.clone(),
                    characteristic: notification.uuid.to_string(),
                    hex: hex(&notification.value),
                });
            }
            Ok(None) => {
                log("Device disconnected".to_string());
                break;
            }
            Err(_) => break,
        }
    }
    let _ = peripheral.disconnect().await;
    log("Done".to_string());
    Ok(())
}
