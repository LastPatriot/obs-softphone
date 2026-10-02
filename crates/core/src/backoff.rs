// SPDX-License-Identifier: GPL-2.0-or-later
//! Registration retry delays: 2 s, 4 s, 8 s, 16 s, then 30 s (DESIGN.md §4.3).

use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Backoff {
    first: Duration,
    max: Duration,
    next: Duration,
}

impl Backoff {
    pub fn new(first: Duration, max: Duration) -> Self {
        Self { first, max, next: first }
    }

    pub fn next_delay(&mut self) -> Duration {
        let delay = self.next;
        self.next = (self.next * 2).min(self.max);
        delay
    }

    pub fn reset(&mut self) {
        self.next = self.first;
    }
}

impl Default for Backoff {
    fn default() -> Self {
        Self::new(Duration::from_secs(2), Duration::from_secs(30))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doubles_up_to_max_then_resets() {
        let mut b = Backoff::default();
        let secs: Vec<u64> = (0..7).map(|_| b.next_delay().as_secs()).collect();
        assert_eq!(secs, [2, 4, 8, 16, 30, 30, 30]);
        b.reset();
        assert_eq!(b.next_delay().as_secs(), 2);
    }
}
