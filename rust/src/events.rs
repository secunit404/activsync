use tokio::sync::broadcast;

/// Broadcasts refresh events to every open page. Late joiners only see
/// future events.
#[derive(Clone)]
pub struct EventBus(broadcast::Sender<&'static str>);

impl EventBus {
    pub fn new() -> Self {
        Self(broadcast::channel(64).0)
    }

    pub fn publish(&self, event: &'static str) {
        let _ = self.0.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<&'static str> {
        self.0.subscribe()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}
