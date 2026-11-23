use crate::{HalError, InputEvent, SystemInputHook, SystemInputInjector};
use std::sync::{Arc, Mutex};

#[derive(Debug, Default, Clone)]
pub struct MockHal {
    pub injected: Arc<Mutex<Vec<InputEvent>>>,
    capturing: Arc<Mutex<bool>>,
}

impl MockHal {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn take_events(&self) -> Vec<InputEvent> {
        let mut guard = self.injected.lock().unwrap();
        std::mem::take(&mut *guard)
    }
}

impl SystemInputInjector for MockHal {
    fn inject(&self, event: InputEvent) -> Result<(), HalError> {
        self.injected.lock().unwrap().push(event);
        Ok(())
    }
}

impl SystemInputHook for MockHal {
    fn start_capture<F>(&mut self, callback: F) -> Result<(), HalError>
    where
        F: Fn(InputEvent) + Send + 'static,
    {
        *self.capturing.lock().unwrap() = true;
        callback(InputEvent::MouseMove { dx: 1, dy: 0 });
        Ok(())
    }

    fn stop_capture(&mut self) {
        *self.capturing.lock().unwrap() = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linux::RawInputEvent;

    #[test]
    fn mock_hal_injects_and_captures() {
        let mut hal = MockHal::new();
        hal.inject_mouse_move(15, -7).unwrap();
        hal.inject_key(30, true).unwrap();
        let events = hal.take_events();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0], InputEvent::MouseMove { dx: 15, dy: -7 });
        hal.start_capture(|_ev| {}).unwrap();
        hal.stop_capture();
    }

    #[test]
    fn linux_raw_input_event_is_24_bytes() {
        let ev = RawInputEvent::new(2, 0, -5);
        assert_eq!(ev.to_bytes().len(), 24);
    }
}
