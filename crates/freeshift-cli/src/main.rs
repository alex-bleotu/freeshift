use std::time::Duration;

use anyhow::{Context, Result, bail};
use freeshift_core::config::{Config, OutputMode};
use freeshift_core::engine::{self, spawn_output};
use freeshift_core::explore;
use freeshift_core::keyboard;
use freeshift_core::model::{Action, EngineEvent};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{broadcast, watch};

const HELP: &str = "FreeShift - Zwift Click / SRAM AXS to MyWhoosh shifting

USAGE:
  freeshift-cli run                          connect controllers and shift (Ctrl+C to stop)
  freeshift-cli scan [seconds]               list nearby Bluetooth LE devices (default 10 s)
  freeshift-cli explore <name> [seconds]     read-only dump of a device's services, then print
                                         every notification while you press buttons (default 60 s)
  freeshift-cli obc-test                     advertise to MyWhoosh and shift from the keyboard:
                                         type u + Enter for up, d + Enter for down, q to quit
  freeshift-cli obc-probe [seconds]          act like a trainer app: find a controller on the network,
                                         connect and print what it sends (default 20 s)
  freeshift-cli key-test <key> [delay]       press <key> in the MyWhoosh window after [delay] s (default 5)
  freeshift-cli config                       print the config file path and contents
  freeshift-cli swap                         swap shift up and shift down for every button
  freeshift-cli output <obc|keys>            choose OpenBikeControl (network) or keyboard output
  freeshift-cli keys <up-key> <down-key>     set keyboard keys, e.g. freeshift-cli keys I K
";

#[tokio::main]
async fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if let Err(error) = run(arguments).await {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

async fn run(arguments: Vec<String>) -> Result<()> {
    let command = arguments.first().map(String::as_str).unwrap_or("help");
    let argument = |index: usize| arguments.get(index).map(String::as_str);
    match command {
        "run" => run_engine().await,
        "scan" => scan(parse_number(argument(1), 10)?).await,
        "explore" => {
            let filter = argument(1).context("give part of the device name, e.g. freeshift-cli explore SRAM")?;
            explore_device(filter, parse_number(argument(2), 60)?).await
        }
        "obc-test" => obc_test().await,
        "obc-probe" => obc_probe(parse_number(argument(1), 20)?).await,
        "key-test" => key_test(argument(1).unwrap_or("I"), parse_number(argument(2), 5)?).await,
        "config" => show_config(),
        "swap" => update_config(|config| config.swap_directions()),
        "output" => {
            let mode = match argument(1) {
                Some("obc") => OutputMode::OpenBikeControl,
                Some("keys") => OutputMode::Keyboard,
                _ => bail!("use: freeshift-cli output obc   or   freeshift-cli output keys"),
            };
            update_config(|config| config.output = mode)
        }
        "keys" => {
            let (Some(up), Some(down)) = (argument(1), argument(2)) else {
                bail!("use: freeshift-cli keys <up-key> <down-key>");
            };
            for key in [up, down] {
                if keyboard::parse_key(key).is_none() {
                    bail!("unknown key \"{key}\"; use a letter, digit, arrow name, Space, PageUp, PageDown or F1-F12");
                }
            }
            update_config(|config| {
                config.keys.insert(Action::ShiftUp, up.to_string());
                config.keys.insert(Action::ShiftDown, down.to_string());
            })
        }
        "help" | "-h" | "--help" => {
            print!("{HELP}");
            Ok(())
        }
        other => bail!("unknown command \"{other}\"\n\n{HELP}"),
    }
}

fn parse_number(value: Option<&str>, default: u64) -> Result<u64> {
    match value {
        Some(text) => text.parse().with_context(|| format!("\"{text}\" is not a number")),
        None => Ok(default),
    }
}

fn print_event(event: &EngineEvent) {
    match event {
        EngineEvent::Log { message } => println!("  {message}"),
        EngineEvent::DeviceConnected { name, kind, id } => println!("+ connected {name} [{kind}] {id}"),
        EngineEvent::DeviceDisconnected { name, .. } => println!("- disconnected {name}"),
        EngineEvent::Battery { id, level } => println!("  battery {id}: {level}%"),
        EngineEvent::Unlocked { id } => println!("  {id} answered the handshake (unlocked)"),
        EngineEvent::Button { device, button, pressed } => {
            println!("  button {} {} ({device})", button.label(), if *pressed { "down" } else { "up" });
        }
        EngineEvent::Action { action, pressed } => {
            if *pressed {
                println!("> {}", action.label());
            }
        }
        EngineEvent::TrainerApp { peer, app, version, supported } => {
            let names: Vec<&str> = supported.iter().map(|action| action.label()).collect();
            println!("  {peer} is {app} {version}, supports: {}", names.join(", "));
        }
        EngineEvent::OutputClient { peer, connected } => {
            println!("{} trainer app {peer}", if *connected { "+ connected" } else { "- disconnected" });
        }
        EngineEvent::OutputReady { description } => println!("  output ready: {description}"),
        EngineEvent::Notification { device, characteristic, hex } => println!("  {device} {characteristic}: {hex}"),
    }
}

fn spawn_printer(events: &broadcast::Sender<EngineEvent>) -> tokio::task::JoinHandle<()> {
    let mut receiver = events.subscribe();
    tokio::spawn(async move {
        loop {
            match receiver.recv().await {
                Ok(event) => print_event(&event),
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

async fn run_engine() -> Result<()> {
    let config = Config::load()?;
    println!("FreeShift running with {} output. Ctrl+C to stop.", output_label(config.output));
    let (events, _) = broadcast::channel(256);
    let printer = spawn_printer(&events);
    let handle = engine::start(config, events.clone());
    tokio::signal::ctrl_c().await.context("waiting for Ctrl+C")?;
    println!("Stopping");
    handle.stop().await;
    drop(events);
    let _ = printer.await;
    Ok(())
}

fn output_label(mode: OutputMode) -> &'static str {
    match mode {
        OutputMode::OpenBikeControl => "OpenBikeControl (network)",
        OutputMode::Keyboard => "keyboard",
    }
}

async fn scan(seconds: u64) -> Result<()> {
    println!("Scanning for {seconds} s. Press a button on each controller to wake it.");
    let devices = explore::scan(seconds).await?;
    if devices.is_empty() {
        println!("No devices found.");
    }
    for device in devices {
        let rssi = device.rssi.map(|value| format!("{value} dBm")).unwrap_or_else(|| "? dBm".to_string());
        let name = if device.name.is_empty() { "(no name)" } else { device.name.as_str() };
        println!("{name}  {}  {rssi}", device.address);
        for line in device.manufacturer_data {
            println!("    manufacturer {line}");
        }
        for line in device.service_data {
            println!("    service data {line}");
        }
        if !device.services.is_empty() {
            println!("    services {}", device.services.join(", "));
        }
    }
    Ok(())
}

async fn explore_device(filter: &str, seconds: u64) -> Result<()> {
    let (events, _) = broadcast::channel(256);
    let printer = spawn_printer(&events);
    let result = explore::explore(filter, seconds, events.clone()).await;
    drop(events);
    let _ = printer.await;
    result
}

async fn obc_test() -> Result<()> {
    let mut config = Config::load()?;
    config.output = OutputMode::OpenBikeControl;
    let (events, _) = broadcast::channel(256);
    let printer = spawn_printer(&events);
    let (stop, stop_receiver) = watch::channel(false);
    let (_settings, settings_receiver) = watch::channel(config.clone());
    let (actions, output) = spawn_output(settings_receiver, events.clone(), stop_receiver);
    println!("In MyWhoosh, add the controller \"{}\". Then type u or d and Enter. q quits.", config.controller_name);
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        let action = match line.trim() {
            "u" => Action::ShiftUp,
            "d" => Action::ShiftDown,
            "q" => break,
            _ => {
                println!("  u = shift up, d = shift down, q = quit");
                continue;
            }
        };
        println!("> {}", action.label());
        let _ = actions.send((action, true));
        tokio::time::sleep(Duration::from_millis(60)).await;
        let _ = actions.send((action, false));
    }
    let _ = stop.send(true);
    let _ = output.await;
    drop(events);
    let _ = printer.await;
    Ok(())
}

async fn obc_probe(seconds: u64) -> Result<()> {
    let (events, _) = broadcast::channel(256);
    let printer = spawn_printer(&events);
    let result = freeshift_core::obc::probe(seconds, events.clone()).await;
    drop(events);
    let _ = printer.await;
    result
}

async fn key_test(name: &str, delay: u64) -> Result<()> {
    let config = Config::load()?;
    let key = keyboard::parse_key(name).with_context(|| format!("unknown key \"{name}\""))?;
    println!("Switch to MyWhoosh. Pressing {name} in {delay} s.");
    tokio::time::sleep(Duration::from_secs(delay)).await;
    if !keyboard::focus_window(&config.focus_window_prefix) {
        println!("No window starting with \"{}\"; pressing in the active window.", config.focus_window_prefix);
    }
    keyboard::send_key(key, true)?;
    tokio::time::sleep(Duration::from_millis(config.key_hold_ms)).await;
    keyboard::send_key(key, false)?;
    println!("Pressed {name}. Did the gear go up or down?");
    Ok(())
}

fn show_config() -> Result<()> {
    let config = Config::load()?;
    println!("{}", Config::path()?.display());
    println!("{}", serde_json::to_string_pretty(&config)?);
    Ok(())
}

fn update_config(change: impl FnOnce(&mut Config)) -> Result<()> {
    let mut config = Config::load()?;
    change(&mut config);
    let path = config.save()?;
    println!("Saved {}", path.display());
    println!("{}", serde_json::to_string_pretty(&config)?);
    Ok(())
}
