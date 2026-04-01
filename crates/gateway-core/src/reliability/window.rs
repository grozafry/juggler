use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

const WINDOW_SIZE: usize = 20;

pub struct WindowedBreaker {
    // Array of success(true) or failure(false).
    outcomes: [AtomicBool; WINDOW_SIZE],
    // Index into the ring buffer.
    cursor: AtomicUsize,
    // Total number of requests we've recorded (caps effectively at WINDOW_SIZE for % calc).
    total_recorded: AtomicUsize,
}

impl WindowedBreaker {
    pub fn new() -> Self {
        const INIT_BOOL: AtomicBool = AtomicBool::new(true); // Default to success initially.
        Self {
            outcomes: [
                INIT_BOOL, INIT_BOOL, INIT_BOOL, INIT_BOOL, INIT_BOOL,
                INIT_BOOL, INIT_BOOL, INIT_BOOL, INIT_BOOL, INIT_BOOL,
                INIT_BOOL, INIT_BOOL, INIT_BOOL, INIT_BOOL, INIT_BOOL,
                INIT_BOOL, INIT_BOOL, INIT_BOOL, INIT_BOOL, INIT_BOOL,
            ],
            cursor: AtomicUsize::new(0),
            total_recorded: AtomicUsize::new(0),
        }
    }

    /// Record a success (true) or failure (false).
    /// Returns true if the active failure rate has crossed the threshold (>50%)
    /// AND we've observed the minimum requests (>=10).
    pub fn record(&self, success: bool) -> bool {
        // We use fetch_add to securely find our slot among threads.
        let idx = self.cursor.fetch_add(1, Ordering::SeqCst) % WINDOW_SIZE;
        self.outcomes[idx].store(success, Ordering::SeqCst);
        
        // Capping at WINDOW_SIZE keeps it from overflowing, while helping us track the minimum 10 constraint.
        let total = self.total_recorded.load(Ordering::Relaxed);
        if total < WINDOW_SIZE {
            self.total_recorded.fetch_add(1, Ordering::Relaxed);
        }

        self.should_trip()
    }

    pub fn should_trip(&self) -> bool {
        let total = self.total_recorded.load(Ordering::SeqCst);
        if total < 10 {
            return false;
        }

        let mut failures = 0;
        let count_to_check = std::cmp::min(total, WINDOW_SIZE);

        for val in self.outcomes.iter().take(count_to_check) {
            if !val.load(Ordering::SeqCst) {
                failures += 1;
            }
        }

        let failure_rate = (failures as f64) / (count_to_check as f64);
        failure_rate > 0.50
    }

    pub fn reset(&self) {
        self.total_recorded.store(0, Ordering::SeqCst);
        self.cursor.store(0, Ordering::SeqCst);
        for item in self.outcomes.iter() {
            item.store(true, Ordering::SeqCst);
        }
    }
}
