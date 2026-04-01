use super::window::WindowedBreaker;
use serde_json::json;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub const CLOSED: u8 = 0;
pub const HALF_OPEN: u8 = 1;
pub const OPEN: u8 = 2;

pub struct CircuitBreaker {
    pub provider_name: String,
    state: AtomicU8,
    cooldown_secs: u64,
    last_tripped: AtomicU64,
    probe_in_flight: AtomicBool,
    window: WindowedBreaker,
    redis_client: Option<redis::Client>,
}

impl CircuitBreaker {
    pub fn new(provider_name: String, cooldown_secs: u64, redis_client: Option<redis::Client>) -> Self {
        Self {
            provider_name,
            state: AtomicU8::new(CLOSED),
            cooldown_secs,
            last_tripped: AtomicU64::new(0),
            probe_in_flight: AtomicBool::new(false),
            window: WindowedBreaker::new(),
            redis_client,
        }
    }

    fn now() -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
    }

    pub fn acquire(&self) -> Result<(), u64> {
        let current_state = self.state.load(Ordering::SeqCst);
        let now_sec = Self::now();

        if current_state == OPEN {
            let tripped_at = self.last_tripped.load(Ordering::SeqCst);
            let elapsed = now_sec.saturating_sub(tripped_at);

            if elapsed >= self.cooldown_secs {
                // Transition to Half-Open and verify if we got the probe slot limit
                if self.state.compare_exchange(OPEN, HALF_OPEN, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
                    self.probe_in_flight.store(true, Ordering::SeqCst);
                    return Ok(());
                }
            }
            let remaining = self.cooldown_secs.saturating_sub(elapsed);
            return Err(remaining);
        }

        if current_state == HALF_OPEN {
            if !self.probe_in_flight.swap(true, Ordering::SeqCst) {
                return Ok(());
            }
            return Err(self.cooldown_secs); // Probe rejected.
        }

        Ok(()) // CLOSED
    }

    pub fn record_success(&self) {
        if self.state.load(Ordering::SeqCst) == HALF_OPEN {
            if self.state.compare_exchange(HALF_OPEN, CLOSED, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
                self.probe_in_flight.store(false, Ordering::SeqCst);
                self.window.reset();
                self.broadcast_event("recovered");
            }
        } else {
            self.window.record(true);
        }
    }

    pub fn record_failure(&self) {
        if self.state.load(Ordering::SeqCst) == HALF_OPEN {
            if self.state.compare_exchange(HALF_OPEN, OPEN, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
                self.last_tripped.store(Self::now(), Ordering::SeqCst);
                self.probe_in_flight.store(false, Ordering::SeqCst);
            }
        } else {
            let over_threshold = self.window.record(false);
            if over_threshold {
                let prev = self.state.swap(OPEN, Ordering::SeqCst);
                if prev != OPEN {
                    self.last_tripped.store(Self::now(), Ordering::SeqCst);
                    self.broadcast_event("tripped");
                }
            }
        }
    }

    pub fn set_open(&self) {
        self.state.store(OPEN, Ordering::SeqCst);
        self.last_tripped.store(Self::now(), Ordering::SeqCst);
    }

    pub fn set_closed(&self) {
        self.state.store(CLOSED, Ordering::SeqCst);
        self.window.reset();
    }

    fn broadcast_event(&self, event: &'static str) {
        if let Some(client) = &self.redis_client {
            let client_cloned = client.clone();
            let provider = self.provider_name.clone();
            tokio::spawn(async move {
                if let Ok(mut conn) = client_cloned.get_multiplexed_async_connection().await {
                    let msg = json!({
                        "event": event,
                        "provider": provider
                    }).to_string();
                    let _: Result<(), _> = redis::cmd("PUBLISH")
                        .arg("gateway:circuit:events")
                        .arg(msg)
                        .query_async(&mut conn)
                        .await;
                }
            });
        }
    }
}
