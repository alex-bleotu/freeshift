use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use anyhow::Result;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::JoinHandle;

use crate::ble::{self, RawInput};
use crate::config::{Config, OutputMode};
use crate::keyboard;
use crate::model::{Action, Button, EngineEvent};
use crate::obc;
use crate::tracker::{ActionTracker, PressTracker};

pub struct EngineHandle {
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl EngineHandle {
    pub async fn stop(self) {
        let _ = self.stop.send(true);
        let _ = self.task.await;
    }
}

pub fn start(config: Config, events: broadcast::Sender<EngineEvent>) -> EngineHandle {
    let (stop, stop_receiver) = watch::channel(false);
    let task = tokio::spawn(async move {
        if let Err(error) = run(config, events.clone(), stop_receiver).await {
            let _ = events.send(EngineEvent::Log { message: format!("{error:#}") });
        }
    });
    EngineHandle { stop, task }
}

pub fn spawn_output(
    config: &Config,
    events: broadcast::Sender<EngineEvent>,
    stop: watch::Receiver<bool>,
) -> (mpsc::UnboundedSender<(Action, bool)>, JoinHandle<()>) {
    let (actions, receiver) = mpsc::unbounded_channel();
    let report = events.clone();
    let task = match config.output {
        OutputMode::OpenBikeControl => {
            let name = config.controller_name.clone();
            let port = config.obc_port;
            tokio::spawn(async move {
                if let Err(error) = obc::run(name, port, receiver, events, stop).await {
                    let _ = report.send(EngineEvent::Log { message: format!("OpenBikeControl output stopped: {error:#}") });
                }
            })
        }
        OutputMode::Keyboard => {
            let mut keys = HashMap::new();
            for (action, name) in [(Action::ShiftUp, &config.shift_up_key), (Action::ShiftDown, &config.shift_down_key)] {
                match keyboard::parse_key(name) {
                    Some(key) => {
                        keys.insert(action, key);
                    }
                    None => {
                        let _ = report.send(EngineEvent::Log { message: format!("Unknown key \"{name}\" for {action:?}") });
                    }
                }
            }
            let hold = Duration::from_millis(config.key_hold_ms);
            let prefix = config.focus_window_prefix.clone();
            let target = if prefix.trim().is_empty() { "the active window".to_string() } else { format!("the \"{prefix}\" window") };
            let description = format!(
                "Keyboard output: {} shifts up, {} shifts down, sent to {target}",
                config.shift_up_key, config.shift_down_key
            );
            tokio::spawn(async move {
                if let Err(error) = keyboard::run(keys, hold, prefix, description, receiver, events, stop).await {
                    let _ = report.send(EngineEvent::Log { message: format!("Keyboard output stopped: {error:#}") });
                }
            })
        }
    };
    (actions, task)
}

async fn run(config: Config, events: broadcast::Sender<EngineEvent>, mut stop: watch::Receiver<bool>) -> Result<()> {
    let (actions, output) = spawn_output(&config, events.clone(), stop.clone());
    let (raw, raw_receiver) = mpsc::unbounded_channel();
    let dispatcher = tokio::spawn(dispatch(config.mapping.clone(), raw_receiver, actions, events.clone()));

    if let Err(error) = ble::run_scanner(config, raw, events.clone(), stop.clone()).await {
        let _ = events.send(EngineEvent::Log { message: format!("Bluetooth: {error:#}") });
        while !*stop.borrow() {
            if stop.changed().await.is_err() {
                break;
            }
        }
    }
    let _ = tokio::time::timeout(Duration::from_secs(5), dispatcher).await;
    let _ = tokio::time::timeout(Duration::from_secs(5), output).await;
    Ok(())
}

pub async fn dispatch(
    mapping: BTreeMap<Button, Action>,
    mut raw: mpsc::UnboundedReceiver<RawInput>,
    actions: mpsc::UnboundedSender<(Action, bool)>,
    events: broadcast::Sender<EngineEvent>,
) {
    let mut presses = PressTracker::default();
    let mut action_state = ActionTracker::default();
    while let Some(input) = raw.recv().await {
        let (device, edges) = match input {
            RawInput::State { device, pressed } => {
                let edges = presses.update(&device, &pressed);
                (device, edges)
            }
            RawInput::Gone { device } => {
                let edges = presses.release_device(&device);
                (device, edges)
            }
            RawInput::Pulse { device, button } => (device, vec![(button, true), (button, false)]),
        };
        for (button, pressed) in edges {
            let _ = events.send(EngineEvent::Button { device: device.clone(), button, pressed });
            let action = mapping.get(&button).copied().unwrap_or_default();
            if let Some((action, pressed)) = action_state.apply(action, pressed) {
                let _ = events.send(EngineEvent::Action { action, pressed });
                let _ = actions.send((action, pressed));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn button_presses_become_shift_actions() {
        let (events, _keep) = broadcast::channel(64);
        let (raw, raw_receiver) = mpsc::unbounded_channel();
        let (actions, mut action_receiver) = mpsc::unbounded_channel();
        let task = tokio::spawn(dispatch(Config::default().mapping, raw_receiver, actions, events));

        raw.send(RawInput::State { device: "left".into(), pressed: vec![Button::Plus] }).unwrap();
        raw.send(RawInput::State { device: "right".into(), pressed: vec![Button::Plus] }).unwrap();
        raw.send(RawInput::State { device: "left".into(), pressed: vec![] }).unwrap();
        raw.send(RawInput::State { device: "right".into(), pressed: vec![Button::Minus] }).unwrap();
        raw.send(RawInput::Gone { device: "right".into() }).unwrap();
        raw.send(RawInput::Pulse { device: "sram".into(), button: Button::DoubleTap }).unwrap();
        raw.send(RawInput::State { device: "left".into(), pressed: vec![Button::A] }).unwrap();
        drop(raw);
        task.await.unwrap();

        let mut received = Vec::new();
        while let Ok(item) = action_receiver.try_recv() {
            received.push(item);
        }
        assert_eq!(
            received,
            vec![
                (Action::ShiftUp, true),
                (Action::ShiftUp, false),
                (Action::ShiftDown, true),
                (Action::ShiftDown, false),
                (Action::ShiftDown, true),
                (Action::ShiftDown, false),
            ]
        );
    }
}
