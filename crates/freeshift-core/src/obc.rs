use std::collections::HashMap;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use mdns_sd::{ServiceDaemon, ServiceInfo};
use socket2::{Domain, Protocol, Socket, Type};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, watch};

use crate::model::{Action, EngineEvent};

pub const SERVICE_TYPE: &str = "_openbikecontrol._tcp.local.";
pub const BLE_SERVICE_UUID: &str = "d273f680-d548-419d-b9d1-fa0472345229";
pub const BUTTON_STATE: u8 = 0x01;
pub const DEVICE_STATUS: u8 = 0x02;

pub const APP_INFO: u8 = 0x04;

pub fn button_message(action: Action, pressed: bool) -> Option<Vec<u8>> {
    let button = action.obc_id()?;
    Some(vec![BUTTON_STATE, button, u8::from(pressed)])
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppInfo {
    pub app: String,
    pub version: String,
    pub supported: Vec<Action>,
    pub supports_all: bool,
}

pub fn parse_app_info(bytes: &[u8]) -> Option<AppInfo> {
    if bytes.first() != Some(&APP_INFO) {
        return None;
    }
    let mut position = 2;
    let text = |position: &mut usize| -> Option<String> {
        let length = usize::from(*bytes.get(*position)?);
        let start = *position + 1;
        let value = bytes.get(start..start + length)?;
        *position = start + length;
        Some(String::from_utf8_lossy(value).into_owned())
    };
    let app = text(&mut position)?;
    let version = text(&mut position)?;
    let count = usize::from(*bytes.get(position)?);
    let ids = bytes.get(position + 1..position + 1 + count)?;
    Some(AppInfo {
        app,
        version,
        supported: ids.iter().filter_map(|id| Action::from_obc_id(*id)).collect(),
        supports_all: count == 0,
    })
}

pub fn status_message() -> Vec<u8> {
    vec![DEVICE_STATUS, 0xFF, 0x01]
}

pub fn host_name() -> String {
    let raw = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "freeshift".to_string());
    let cleaned: String = raw
        .chars()
        .map(|character| if character.is_ascii_alphanumeric() { character.to_ascii_lowercase() } else { '-' })
        .collect();
    format!("{cleaned}-freeshift.local.")
}

pub fn instance_id(name: &str) -> String {
    let host = std::env::var("COMPUTERNAME").unwrap_or_default();
    format!("{}-{}", host.to_ascii_lowercase(), name.to_ascii_lowercase().replace(' ', "-"))
}

fn bind_listener(port: u16) -> std::io::Result<TcpListener> {
    let dual_stack = || -> std::io::Result<std::net::TcpListener> {
        let socket = Socket::new(Domain::IPV6, Type::STREAM, Some(Protocol::TCP))?;
        socket.set_only_v6(false)?;
        socket.bind(&SocketAddr::from((Ipv6Addr::UNSPECIFIED, port)).into())?;
        socket.listen(128)?;
        Ok(socket.into())
    };
    let listener = match dual_stack() {
        Ok(listener) => listener,
        Err(_) => std::net::TcpListener::bind((Ipv4Addr::UNSPECIFIED, port))?,
    };
    listener.set_nonblocking(true)?;
    TcpListener::from_std(listener)
}

pub async fn run(
    name: String,
    port: u16,
    mut actions: mpsc::UnboundedReceiver<(Action, bool)>,
    events: broadcast::Sender<EngineEvent>,
    mut stop: watch::Receiver<bool>,
) -> Result<()> {
    let listener = bind_listener(port).with_context(|| format!("port {port} is in use; pick another in settings"))?;
    let bound_port = listener.local_addr()?.port();

    let daemon = ServiceDaemon::new().context("starting mDNS")?;
    let properties = HashMap::from([
        ("version".to_string(), "1".to_string()),
        ("id".to_string(), instance_id(&name)),
        ("name".to_string(), name.clone()),
        ("service-uuids".to_string(), BLE_SERVICE_UUID.to_string()),
        ("manufacturer".to_string(), "FreeShift".to_string()),
        ("model".to_string(), "FreeShift".to_string()),
    ]);
    let info = ServiceInfo::new(SERVICE_TYPE, &name, &host_name(), (), bound_port, properties)
        .context("building mDNS record")?
        .enable_addr_auto();
    let fullname = info.get_fullname().to_string();
    daemon.register(info).context("announcing on mDNS")?;
    let _ = events.send(EngineEvent::OutputReady {
        description: format!("OpenBikeControl controller \"{name}\" on TCP port {bound_port}"),
    });

    let (outgoing, _) = broadcast::channel::<Vec<u8>>(64);
    let mut status_timer = tokio::time::interval(Duration::from_secs(30));

    loop {
        tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    break;
                }
            }
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, peer)) => {
                        let peer = peer.to_string();
                        let _ = events.send(EngineEvent::OutputClient { peer: peer.clone(), connected: true });
                        tokio::spawn(serve_client(stream, peer, outgoing.subscribe(), events.clone(), stop.clone()));
                    }
                    Err(error) => {
                        let _ = events.send(EngineEvent::Log { message: format!("TCP accept failed: {error}") });
                    }
                }
            }
            action = actions.recv() => {
                let Some((action, pressed)) = action else {
                    break;
                };
                if let Some(message) = button_message(action, pressed) {
                    let _ = outgoing.send(message);
                }
            }
            _ = status_timer.tick() => {
                let _ = outgoing.send(status_message());
            }
        }
    }

    if let Ok(receiver) = daemon.unregister(&fullname) {
        let _ = receiver.recv_timeout(Duration::from_secs(1));
    }
    if let Ok(receiver) = daemon.shutdown() {
        let _ = receiver.recv_timeout(Duration::from_secs(1));
    }
    Ok(())
}

async fn serve_client(
    stream: TcpStream,
    peer: String,
    mut outgoing: broadcast::Receiver<Vec<u8>>,
    events: broadcast::Sender<EngineEvent>,
    mut stop: watch::Receiver<bool>,
) {
    let _ = stream.set_nodelay(true);
    let (mut reader, mut writer) = stream.into_split();
    let mut buffer = [0u8; 256];
    let mut reason = String::from("closed by app");

    if writer.write_all(&status_message()).await.is_err() {
        reason = "write failed".to_string();
    } else {
        loop {
            tokio::select! {
                changed = stop.changed() => {
                    if changed.is_err() || *stop.borrow() {
                        reason = "stopped".to_string();
                        break;
                    }
                }
                message = outgoing.recv() => {
                    match message {
                        Ok(bytes) => {
                            if let Err(error) = writer.write_all(&bytes).await {
                                reason = format!("write failed: {error}");
                                break;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            let _ = events.send(EngineEvent::Log { message: format!("{peer} skipped {skipped} messages") });
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
                read = reader.read(&mut buffer) => {
                    match read {
                        Ok(0) => break,
                        Ok(count) => {
                            let bytes = &buffer[..count];
                            match parse_app_info(bytes) {
                                Some(info) => {
                                    let supported = if info.supports_all { Action::ALL.to_vec() } else { info.supported };
                                    let _ = events.send(EngineEvent::TrainerApp {
                                        peer: peer.clone(),
                                        app: info.app,
                                        version: info.version,
                                        supported,
                                    });
                                }
                                None => {
                                    let _ = events.send(EngineEvent::Log {
                                        message: format!("{peer} sent {}", crate::model::hex(bytes)),
                                    });
                                }
                            }
                        }
                        Err(error) => {
                            reason = format!("read failed: {error}");
                            break;
                        }
                    }
                }
            }
        }
    }

    let _ = events.send(EngineEvent::Log { message: format!("{peer} disconnected ({reason})") });
    let _ = events.send(EngineEvent::OutputClient { peer, connected: false });
}

pub async fn probe(seconds: u64, events: broadcast::Sender<EngineEvent>) -> Result<()> {
    let log = |message: String| {
        let _ = events.send(EngineEvent::Log { message });
    };
    log(format!("Browsing {SERVICE_TYPE} for up to {seconds} s"));
    let wait = Duration::from_secs(seconds);
    let found = tokio::task::spawn_blocking(move || -> Result<Option<(String, Vec<SocketAddr>)>> {
        let daemon = ServiceDaemon::new().context("starting mDNS")?;
        let receiver = daemon.browse(SERVICE_TYPE).context("browsing mDNS")?;
        let started = std::time::Instant::now();
        let mut result: Option<(String, Vec<SocketAddr>)> = None;
        while started.elapsed() < wait {
            let Ok(event) = receiver.recv_timeout(Duration::from_millis(250)) else {
                if result.is_some() {
                    break;
                }
                continue;
            };
            if let mdns_sd::ServiceEvent::ServiceResolved(service) = event {
                let entry = result.get_or_insert_with(|| (service.fullname.clone(), Vec::new()));
                for address in service.addresses.iter().map(|scoped| SocketAddr::new(scoped.to_ip_addr(), service.port)) {
                    if !entry.1.contains(&address) {
                        entry.1.push(address);
                    }
                }
                entry.1.sort_by_key(|address| !address.is_ipv4());
            }
        }
        let _ = daemon.shutdown();
        Ok(result)
    })
    .await??;

    let Some((fullname, addresses)) = found else {
        log("No OpenBikeControl controller found on the network".to_string());
        return Ok(());
    };
    let listed: Vec<String> = addresses.iter().map(ToString::to_string).collect();
    log(format!("Found {fullname} at {}", listed.join(", ")));
    let mut connected = None;
    for address in &addresses {
        match tokio::time::timeout(Duration::from_secs(3), TcpStream::connect(address)).await {
            Ok(Ok(stream)) => {
                log(format!("Connected to {address} like a trainer app would"));
                connected = Some(stream);
                break;
            }
            Ok(Err(error)) => log(format!("{address}: {error}")),
            Err(_) => log(format!("{address}: timed out")),
        }
    }
    let Some(mut stream) = connected else {
        bail!("could not connect to any advertised address");
    };
    let mut buffer = [0u8; 256];
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        match tokio::time::timeout_at(deadline, stream.read(&mut buffer)).await {
            Ok(Ok(0)) => {
                log("Controller closed the connection".to_string());
                break;
            }
            Ok(Ok(count)) => log(format!("received {}", crate::model::hex(&buffer[..count]))),
            Ok(Err(error)) => {
                log(format!("read failed: {error}"));
                break;
            }
            Err(_) => break,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_shift_messages() {
        assert_eq!(button_message(Action::ShiftUp, true), Some(vec![0x01, 0x01, 0x01]));
        assert_eq!(button_message(Action::ShiftUp, false), Some(vec![0x01, 0x01, 0x00]));
        assert_eq!(button_message(Action::ShiftDown, true), Some(vec![0x01, 0x02, 0x01]));
        assert_eq!(button_message(Action::ShiftDown, false), Some(vec![0x01, 0x02, 0x00]));
        assert_eq!(button_message(Action::Menu, true), Some(vec![0x01, 0x16, 0x01]));
        assert_eq!(button_message(Action::Select, false), Some(vec![0x01, 0x14, 0x00]));
        assert_eq!(button_message(Action::None, true), None);
    }

    #[test]
    fn parses_the_spec_app_info_example() {
        let mut bytes = vec![0x04, 0x01, 0x05];
        bytes.extend(b"zwift");
        bytes.push(0x06);
        bytes.extend(b"1.52.0");
        bytes.extend([0x04, 0x01, 0x02, 0x10, 0x14]);
        let info = parse_app_info(&bytes).unwrap();
        assert_eq!(info.app, "zwift");
        assert_eq!(info.version, "1.52.0");
        assert_eq!(info.supported, vec![Action::ShiftUp, Action::ShiftDown, Action::Up, Action::Select]);
        assert!(!info.supports_all);
    }

    #[test]
    fn app_info_with_zero_buttons_means_all() {
        let bytes = [0x04, 0x01, 0x02, b'm', b'w', 0x01, b'5', 0x00];
        assert!(parse_app_info(&bytes).unwrap().supports_all);
        assert_eq!(parse_app_info(&[0x04, 0x01, 0x09, b'x']), None);
        assert_eq!(parse_app_info(&[0x01, 0x01, 0x01]), None);
    }

    #[test]
    fn host_name_is_mdns_safe() {
        let host = host_name();
        assert!(host.ends_with(".local."));
        assert!(host.trim_end_matches(".local.").chars().all(|character| character.is_ascii_alphanumeric() || character == '-'));
    }
}
