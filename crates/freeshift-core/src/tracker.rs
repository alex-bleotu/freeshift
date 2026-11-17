use std::collections::{HashMap, HashSet};

use crate::model::{Action, Button};

#[derive(Default)]
pub struct PressTracker {
    holders: HashMap<Button, HashSet<String>>,
}

impl PressTracker {
    pub fn update(&mut self, device: &str, pressed: &[Button]) -> Vec<(Button, bool)> {
        let mut edges = Vec::new();
        for button in Button::ALL {
            let holders = self.holders.entry(button).or_default();
            let was_down = !holders.is_empty();
            if pressed.contains(&button) {
                holders.insert(device.to_string());
            } else {
                holders.remove(device);
            }
            let is_down = !holders.is_empty();
            if was_down != is_down {
                edges.push((button, is_down));
            }
        }
        edges
    }

    pub fn release_device(&mut self, device: &str) -> Vec<(Button, bool)> {
        self.update(device, &[])
    }
}

#[derive(Default)]
pub struct ActionTracker {
    counts: HashMap<Action, usize>,
}

impl ActionTracker {
    pub fn apply(&mut self, action: Action, pressed: bool) -> Option<(Action, bool)> {
        if action == Action::None {
            return None;
        }
        let count = self.counts.entry(action).or_default();
        if pressed {
            *count += 1;
            if *count == 1 {
                return Some((action, true));
            }
            return None;
        }
        if *count == 0 {
            return None;
        }
        *count -= 1;
        if *count == 0 {
            return Some((action, false));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_device_press_and_release() {
        let mut tracker = PressTracker::default();
        assert_eq!(tracker.update("a", &[Button::Plus]), vec![(Button::Plus, true)]);
        assert_eq!(tracker.update("a", &[Button::Plus]), vec![]);
        assert_eq!(tracker.update("a", &[]), vec![(Button::Plus, false)]);
    }

    #[test]
    fn mirrored_frames_from_two_pods_fire_once() {
        let mut tracker = PressTracker::default();
        assert_eq!(tracker.update("left", &[Button::Plus]), vec![(Button::Plus, true)]);
        assert_eq!(tracker.update("right", &[Button::Plus]), vec![]);
        assert_eq!(tracker.update("left", &[]), vec![]);
        assert_eq!(tracker.update("right", &[]), vec![(Button::Plus, false)]);
    }

    #[test]
    fn disconnect_releases_held_buttons() {
        let mut tracker = PressTracker::default();
        tracker.update("a", &[Button::Minus, Button::B]);
        let mut edges = tracker.release_device("a");
        edges.sort();
        assert_eq!(edges, vec![(Button::Minus, false), (Button::B, false)]);
    }

    #[test]
    fn two_buttons_on_same_action_send_one_press() {
        let mut tracker = ActionTracker::default();
        assert_eq!(tracker.apply(Action::ShiftUp, true), Some((Action::ShiftUp, true)));
        assert_eq!(tracker.apply(Action::ShiftUp, true), None);
        assert_eq!(tracker.apply(Action::ShiftUp, false), None);
        assert_eq!(tracker.apply(Action::ShiftUp, false), Some((Action::ShiftUp, false)));
        assert_eq!(tracker.apply(Action::ShiftUp, false), None);
        assert_eq!(tracker.apply(Action::None, true), None);
    }
}
