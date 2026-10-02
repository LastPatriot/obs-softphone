// SPDX-License-Identifier: GPL-2.0-or-later
//! The return feed's buffer (DESIGN.md §3.3): OBS's audio thread writes the
//! chosen track (1024-sample chunks), PJSIP's media clock reads 20 ms frames.
//!
//! Single producer, single consumer, lock-free. Both sides run on clocks
//! derived from the system clock, so drift is tiny; instead of resampling,
//! the reader keeps the delay bounded:
//! - it starts (and restarts after running dry) only once `TARGET` is
//!   queued, then outputs silence for whatever is missing;
//! - if more than `HIGH` is queued (e.g. stale audio from before the call),
//!   it drops the oldest down to `TARGET`.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

/// 48 kHz mono.
const MS: usize = 48;
pub const CAPACITY: usize = 250 * MS;
pub const TARGET: usize = 40 * MS;
pub const HIGH: usize = 120 * MS;

pub struct ReturnFeed {
    buf: Box<[UnsafeCell<i16>]>,
    /// Total samples ever written / read; the difference is the fill.
    written: AtomicUsize,
    read: AtomicUsize,
    primed: AtomicBool,
    underruns: AtomicU64,
    skipped: AtomicU64,
}

// SAFETY: the producer only writes slots outside [read, written) and
// publishes them with Release on `written`; the consumer only reads slots
// inside it after an Acquire load, and frees them with Release on `read`.
unsafe impl Sync for ReturnFeed {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FeedStats {
    pub queued: usize,
    pub underruns: u64,
    pub skipped: u64,
}

impl Default for ReturnFeed {
    fn default() -> Self {
        Self {
            buf: (0..CAPACITY).map(|_| UnsafeCell::new(0)).collect(),
            written: AtomicUsize::new(0),
            read: AtomicUsize::new(0),
            primed: AtomicBool::new(false),
            underruns: AtomicU64::new(0),
            skipped: AtomicU64::new(0),
        }
    }
}

impl ReturnFeed {
    /// Producer (OBS audio thread). Samples that don't fit are dropped.
    pub fn push(&self, samples: &[i16]) {
        let w = self.written.load(Ordering::Relaxed);
        let r = self.read.load(Ordering::Acquire);
        let n = samples.len().min(CAPACITY - (w - r));
        for (i, &s) in samples[..n].iter().enumerate() {
            // SAFETY: slot w+i is free (see the Sync impl).
            unsafe { *self.buf[(w + i) % CAPACITY].get() = s };
        }
        self.written.store(w + n, Ordering::Release);
    }

    /// Consumer (PJSIP media clock). Always fills `out` completely.
    pub fn pop(&self, out: &mut [i16]) {
        let mut r = self.read.load(Ordering::Relaxed);
        let w = self.written.load(Ordering::Acquire);
        let mut avail = w - r;

        if avail > HIGH {
            let skip = avail - TARGET;
            r += skip;
            avail = TARGET;
            self.skipped.fetch_add(skip as u64, Ordering::Relaxed);
        }
        if !self.primed.load(Ordering::Relaxed) {
            if avail < TARGET {
                out.fill(0);
                self.read.store(r, Ordering::Release);
                return;
            }
            self.primed.store(true, Ordering::Relaxed);
        }

        let n = out.len().min(avail);
        for (i, o) in out[..n].iter_mut().enumerate() {
            // SAFETY: slot r+i is filled (see the Sync impl).
            *o = unsafe { *self.buf[(r + i) % CAPACITY].get() };
        }
        if n < out.len() {
            out[n..].fill(0);
            self.underruns.fetch_add(1, Ordering::Relaxed);
            self.primed.store(false, Ordering::Relaxed);
        }
        self.read.store(r + n, Ordering::Release);
    }

    pub fn stats(&self) -> FeedStats {
        let w = self.written.load(Ordering::Acquire);
        let r = self.read.load(Ordering::Acquire);
        FeedStats {
            queued: w.saturating_sub(r),
            underruns: self.underruns.load(Ordering::Relaxed),
            skipped: self.skipped.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(start: i16, n: usize) -> Vec<i16> {
        (0..n).map(|i| start.wrapping_add(i as i16)).collect()
    }

    #[test]
    fn waits_for_target_then_plays_in_order() {
        let f = ReturnFeed::default();
        let mut out = vec![1; 960];
        f.push(&ramp(0, 1024));
        f.pop(&mut out);
        assert!(out.iter().all(|&s| s == 0), "not primed yet: silence");
        assert_eq!(f.stats().queued, 1024, "nothing consumed while priming");

        f.push(&ramp(1024, 1024)); // 2048 ≥ TARGET (1920)
        f.pop(&mut out);
        assert_eq!(out, ramp(0, 960));
        f.pop(&mut out);
        assert_eq!(out, ramp(960, 960));
        assert_eq!(f.stats().underruns, 0);
    }

    #[test]
    fn underrun_pads_with_silence_and_reprimes() {
        let f = ReturnFeed::default();
        f.push(&ramp(1, TARGET));
        let mut out = vec![0; 960];
        f.pop(&mut out);
        f.pop(&mut out);
        f.pop(&mut out); // only 0 left of 960
        assert!(out.iter().all(|&s| s == 0));
        assert_eq!(f.stats().underruns, 1);

        f.push(&ramp(5, 960)); // below TARGET: still silence
        f.pop(&mut out);
        assert!(out.iter().all(|&s| s == 0));
        f.push(&ramp(965, 960));
        f.pop(&mut out);
        assert_eq!(out, ramp(5, 960));
    }

    #[test]
    fn partial_frame_is_padded() {
        let f = ReturnFeed::default();
        f.push(&ramp(1, TARGET + 100));
        let mut out = vec![0; 960];
        f.pop(&mut out);
        f.pop(&mut out);
        f.pop(&mut out); // 100 left
        assert_eq!(&out[..100], &ramp(1 + 1920, 100)[..]);
        assert!(out[100..].iter().all(|&s| s == 0));
    }

    #[test]
    fn too_much_queued_drops_the_oldest() {
        let f = ReturnFeed::default();
        f.push(&ramp(0, 10_000)); // e.g. filled while no call was active
        let mut out = vec![0; 960];
        f.pop(&mut out);
        // Kept the newest TARGET samples.
        assert_eq!(out, ramp((10_000 - TARGET) as i16, 960));
        assert_eq!(f.stats().skipped, (10_000 - TARGET) as u64);
    }

    #[test]
    fn full_buffer_drops_new_samples_and_wraps_correctly() {
        let f = ReturnFeed::default();
        f.push(&ramp(0, CAPACITY + 500));
        assert_eq!(f.stats().queued, CAPACITY);
        let mut out = vec![0; 960];
        f.pop(&mut out); // skips to the newest TARGET of what fit
        assert_eq!(out, ramp((CAPACITY - TARGET) as i16, 960));
        // Keep going across the wrap point.
        for k in 0..20 {
            f.push(&ramp(k * 960, 960));
            f.pop(&mut out);
        }
        assert_eq!(f.stats().underruns, 0);
    }

    #[test]
    fn threads_keep_order_and_bounded_delay() {
        use std::sync::Arc;
        let f = Arc::new(ReturnFeed::default());
        let p = f.clone();
        let producer = std::thread::spawn(move || {
            let mut next: i16 = 0;
            for _ in 0..2000 {
                let chunk: Vec<i16> = (0..1024).map(|_| { next = next.wrapping_add(1); next }).collect();
                p.push(&chunk);
                std::thread::yield_now();
            }
        });
        let mut out = vec![0; 960];
        let mut last: Option<i16> = None;
        for _ in 0..2000 {
            f.pop(&mut out);
            for &s in out.iter().filter(|&&s| s != 0) {
                if let Some(prev) = last {
                    // Monotonic except where the reader deliberately skipped.
                    assert!(s.wrapping_sub(prev) > 0, "out of order: {prev} then {s}");
                }
                last = Some(s);
            }
            assert!(f.stats().queued <= CAPACITY);
            std::thread::yield_now();
        }
        producer.join().unwrap();
    }
}
