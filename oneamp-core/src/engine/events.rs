//! Bounded event queue with merging of replaceable updates.

use super::AudioEvent;
use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

/// Events that only matter in their latest version: a queued one is
/// replaced in place instead of piling up behind a slow consumer.
fn replaceable(e: &AudioEvent) -> bool {
    matches!(
        e,
        AudioEvent::Position { .. }
            | AudioEvent::VolumeUpdated(..)
            | AudioEvent::BalanceUpdated(..)
            | AudioEvent::EqualizerUpdated(..)
            | AudioEvent::EqualizerPreampUpdated(..)
            | AudioEvent::StreamReconnecting { .. }
    )
}

struct Inner {
    queue: VecDeque<AudioEvent>,
    /// Dropped and not yet reported.
    dropped: u64,
    total_dropped: u64,
}

/// Fixed-capacity queue. Pushing never blocks and memory never grows
/// past `capacity` events.
pub struct EventQueue {
    inner: Mutex<Inner>,
    ready: Condvar,
    capacity: usize,
}

impl EventQueue {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(Inner {
                queue: VecDeque::with_capacity(capacity),
                dropped: 0,
                total_dropped: 0,
            }),
            ready: Condvar::new(),
            capacity,
        }
    }

    pub fn push(&self, event: AudioEvent) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if replaceable(&event) {
            let d = std::mem::discriminant(&event);
            if let Some(slot) = g.queue.iter_mut().find(|e| std::mem::discriminant(*e) == d) {
                *slot = event;
                return;
            }
        }
        if g.queue.len() >= self.capacity {
            g.queue.pop_front();
            g.dropped += 1;
            g.total_dropped += 1;
        }
        g.queue.push_back(event);
        drop(g);
        self.ready.notify_one();
    }

    /// Events dropped and not reported yet.
    pub fn dropped(&self) -> u64 {
        self.inner.lock().map(|g| g.dropped).unwrap_or(0)
    }

    /// Events dropped since creation.
    pub fn total_dropped(&self) -> u64 {
        self.inner.lock().map(|g| g.total_dropped).unwrap_or(0)
    }

    pub fn len(&self) -> usize {
        self.inner.lock().map(|g| g.queue.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn try_pop(&self) -> Option<AudioEvent> {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        Self::pop_locked(&mut g)
    }

    fn pop_locked(g: &mut Inner) -> Option<AudioEvent> {
        // Report losses before what follows them.
        if g.dropped > 0 && !matches!(g.queue.front(), Some(AudioEvent::EventsDropped(_))) {
            let n = std::mem::take(&mut g.dropped);
            return Some(AudioEvent::EventsDropped(n));
        }
        g.queue.pop_front()
    }

    pub fn pop_timeout(&self, timeout: Duration) -> Option<AudioEvent> {
        let deadline = std::time::Instant::now() + timeout;
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(e) = Self::pop_locked(&mut g) {
                return Some(e);
            }
            let now = std::time::Instant::now();
            if now >= deadline {
                return None;
            }
            g = self
                .ready
                .wait_timeout(g, deadline - now)
                .map(|(g, _)| g)
                .unwrap_or_else(|e| e.into_inner().0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaceable_updates_merge_and_memory_stays_bounded() {
        let q = EventQueue::new(8);
        for i in 0..10_000 {
            q.push(AudioEvent::Position {
                position: Duration::from_millis(i),
                duration: None,
            });
            q.push(AudioEvent::VolumeUpdated(i as f32, false));
        }
        assert_eq!(q.len(), 2);
        match q.try_pop() {
            Some(AudioEvent::Position { position, .. }) => {
                assert_eq!(position, Duration::from_millis(9_999))
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn overflow_drops_oldest_and_reports_it() {
        let q = EventQueue::new(4);
        for _ in 0..10 {
            q.push(AudioEvent::Playing);
        }
        assert_eq!(q.len(), 4);
        assert!(matches!(q.try_pop(), Some(AudioEvent::EventsDropped(6))));
        assert!(matches!(q.try_pop(), Some(AudioEvent::Playing)));
        assert_eq!(q.dropped(), 0);
    }

    #[test]
    fn pop_timeout_wakes_on_push() {
        let q = std::sync::Arc::new(EventQueue::new(4));
        let q2 = q.clone();
        let t = std::thread::spawn(move || q2.pop_timeout(Duration::from_secs(5)));
        std::thread::sleep(Duration::from_millis(20));
        q.push(AudioEvent::Stopped);
        assert!(matches!(t.join().unwrap(), Some(AudioEvent::Stopped)));
        assert!(q.pop_timeout(Duration::from_millis(10)).is_none());
    }
}
