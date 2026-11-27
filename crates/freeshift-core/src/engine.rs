use std::collections::HashMap;
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
    settings: watch::Sender<Config>,
    task: JoinHandle<()>,
}

impl EngineHandle {
    pub fn apply(&self, config: &Config) -> bool {
        if self.task.is_finished() || !self.settings.borrow().can_apply_live(config) {
            return false;
        }
        self.settings.send_replace(config.clone());
        true
    }

    pub async fn stop(self) {
        let _ = self.stop.send(true);
        let _ = self.task.await;
    }
}

pub fn start(config: Config, events: broadcast::Sender<EngineEvent>) -> EngineHandle {
    let (stop, stop_receiver) = watch::channel(false);
    let (settings, settings_receiver) = watch::channel(config);
    let task = tokio::spawn(async move {
        if let Err(error) = run(settings_receiver, events.clone(), stop_receiver).await {
            let _ = events.send(EngineEvent::Log { message: format!("{error:#}") });
        }
    });
    EngineHandle { stop, settings, task }
}

pub fn spawn_output(
    settings: watch::Receiver<Config>,
    events: broadcast::Sender<EngineEvent>,
    stop: watch::Receiver<bool>,
) -> (mpsc::UnboundedSender<(Action, bool)>, JoinHandle<()>) {
    let (actions, receiver) = mpsc::unbounded_channel();
    let report = events.clone();
    let config = settings.borrow().clone();
    let task = match config.output {
        OutputMode::OpenBikeControl => {
            let name = config.controller_name;
            let port = config.obc_port;
            tokio::spawn(async move {
                if let Err(error) = obc::run(name, port, receiver, events, stop).await {
                    let _ = report.send(EngineEvent::Log { message: format!("OpenBikeControl output stopped: {error:#}") });
                }
            })
        }
        OutputMode::Keyboard => tokio::spawn(async move {
            if let Err(error) = keyboard::run(settings, receiver, events, stop).await {
                let _ = report.send(EngineEvent::Log { message: format!("Keyboard output stopped: {error:#}") });
            }
        }),
    };
    (actions, task)
}

async fn run(
    settings: watch::Receiver<Config>,
    events: broadcast::Sender<EngineEvent>,
    mut stop: watch::Receiver<bool>,
) -> Result<()> {
    let (actions, output) = spawn_output(settings.clone(), events.clone(), stop.clone());
    let (raw, raw_receiver) = mpsc::unbounded_channel();
    let dispatcher = tokio::spawn(dispatch(settings.clone(), raw_receiver, actions, events.clone()));

    if let Err(error) = ble::run_scanner(settings, raw, events.clone(), stop.clone()).await {
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
    settings: watch::Receiver<Config>,
    mut raw: mpsc::UnboundedReceiver<RawInput>,
    actions: mpsc::UnboundedSender<(Action, bool)>,
    events: broadcast::Sender<EngineEvent>,
) {
    let mut presses = PressTracker::default();
    let mut action_state = ActionTracker::default();
    let mut held: HashMap<Button, Action> = HashMap::new();
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
            let action = if pressed {
                let action = settings.borrow().action_for(button);
                held.insert(button, action);
                action
            } else {
                held.remove(&button).unwrap_or_default()
            };
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

    fn settings_with(mapping: &[(Button, Action)]) -> (watch::Sender<Config>, watch::Receiver<Config>) {
        watch::channel(Config { mapping: mapping.iter().copied().collect(), ..Config::default() })
    }

    fn idle_engine(config: Config) -> EngineHandle {
        let (stop, mut stop_receiver) = watch::channel(false);
        let (settings, _) = watch::channel(config);
        let task = tokio::spawn(async move {
            let _ = stop_receiver.changed().await;
        });
        EngineHandle { stop, settings, task }
    }

    #[tokio::test]
    async fn a_running_engine_takes_button_changes_and_refuses_connection_changes() {
        let current = Config::default();
        let handle = idle_engine(current.clone());
        let mut remapped = current.clone();
        remapped.mapping.insert(Button::Plus, Action::Pause);
        assert!(handle.apply(&remapped));
        assert_eq!(*handle.settings.borrow(), remapped);

        let moved = Config { obc_port: 21588, ..remapped.clone() };
        assert!(!handle.apply(&moved));
        assert_eq!(*handle.settings.borrow(), remapped);
        handle.stop().await;
    }

    #[tokio::test]
    async fn a_finished_engine_is_never_updated_in_place() {
        let mut handle = idle_engine(Config::default());
        handle.stop.send_replace(true);
        (&mut handle.task).await.unwrap();
        assert!(!handle.apply(&Config::default()));
    }

    #[tokio::test]
    async fn a_new_action_applies_to_the_next_press_and_not_to_a_held_button() {
        let (events, _keep) = broadcast::channel(64);
        let (raw, raw_receiver) = mpsc::unbounded_channel();
        let (actions, mut received) = mpsc::unbounded_channel();
        let (settings, settings_receiver) = settings_with(&[(Button::Plus, Action::ShiftUp)]);
        let task = tokio::spawn(dispatch(settings_receiver, raw_receiver, actions, events));

        raw.send(RawInput::State { device: "left".into(), pressed: vec![Button::Plus] }).unwrap();
        assert_eq!(received.recv().await, Some((Action::ShiftUp, true)));

        settings.send_modify(|config| {
            config.mapping.insert(Button::Plus, Action::Pause);
        });
        raw.send(RawInput::State { device: "left".into(), pressed: vec![] }).unwrap();
        assert_eq!(received.recv().await, Some((Action::ShiftUp, false)));

        raw.send(RawInput::State { device: "left".into(), pressed: vec![Button::Plus] }).unwrap();
        raw.send(RawInput::State { device: "left".into(), pressed: vec![] }).unwrap();
        assert_eq!(received.recv().await, Some((Action::Pause, true)));
        assert_eq!(received.recv().await, Some((Action::Pause, false)));

        drop(raw);
        task.await.unwrap();
        assert_eq!(received.recv().await, None);
    }

    #[tokio::test]
    async fn an_action_given_to_a_held_button_waits_for_the_next_press() {
        let (events, _keep) = broadcast::channel(64);
        let (raw, raw_receiver) = mpsc::unbounded_channel();
        let (actions, mut received) = mpsc::unbounded_channel();
        let (settings, settings_receiver) = settings_with(&[(Button::Plus, Action::ShiftUp)]);
        let task = tokio::spawn(dispatch(settings_receiver, raw_receiver, actions, events));

        raw.send(RawInput::State { device: "left".into(), pressed: vec![Button::A, Button::Plus] }).unwrap();
        assert_eq!(received.recv().await, Some((Action::ShiftUp, true)));
        settings.send_modify(|config| {
            config.mapping.insert(Button::A, Action::Select);
            config.mapping.remove(&Button::Plus);
        });
        raw.send(RawInput::Gone { device: "left".into() }).unwrap();
        drop(raw);
        task.await.unwrap();
        assert_eq!(received.recv().await, Some((Action::ShiftUp, false)));
        assert_eq!(received.recv().await, None);
    }

    #[tokio::test]
    async fn button_presses_become_shift_actions() {
        let (events, _keep) = broadcast::channel(64);
        let (raw, raw_receiver) = mpsc::unbounded_channel();
        let (actions, mut action_receiver) = mpsc::unbounded_channel();
        let (_settings, settings_receiver) = settings_with(&[
            (Button::Plus, Action::ShiftUp),
            (Button::Minus, Action::ShiftDown),
            (Button::SramLeft, Action::ShiftDown),
        ]);
        let task = tokio::spawn(dispatch(settings_receiver, raw_receiver, actions, events));

        raw.send(RawInput::State { device: "left".into(), pressed: vec![Button::Plus] }).unwrap();
        raw.send(RawInput::State { device: "right".into(), pressed: vec![Button::Plus] }).unwrap();
        raw.send(RawInput::State { device: "left".into(), pressed: vec![] }).unwrap();
        raw.send(RawInput::State { device: "right".into(), pressed: vec![Button::Minus] }).unwrap();
        raw.send(RawInput::Gone { device: "right".into() }).unwrap();
        raw.send(RawInput::Pulse { device: "sram".into(), button: Button::SramLeft }).unwrap();
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
