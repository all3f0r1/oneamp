//! A minimal play queue for hosts that do not have one: an item list,
//! an order (identity, or a shuffle the host already decided), a
//! position and the repeat policy. Hosts with their own queue
//! (Tesserafin) skip this and only tell the engine what comes next.

use crate::settings::RepeatMode;

#[derive(Debug, Clone)]
pub struct PlayQueue<T> {
    items: Vec<T>,
    /// Play order as indices into `items`.
    order: Vec<usize>,
    /// Position in `order`.
    pos: Option<usize>,
    repeat: RepeatMode,
}

impl<T> PlayQueue<T> {
    pub fn new(items: Vec<T>) -> Self {
        let order = (0..items.len()).collect();
        Self {
            items,
            order,
            pos: None,
            repeat: RepeatMode::Off,
        }
    }

    /// Use a predetermined order (e.g. the host's shuffle). Must be a
    /// permutation of `0..len`; otherwise the order is left unchanged
    /// and `false` returned.
    pub fn set_order(&mut self, order: Vec<usize>) -> bool {
        let mut seen = vec![false; self.items.len()];
        let valid = order.len() == self.items.len()
            && order
                .iter()
                .all(|&i| i < seen.len() && !std::mem::replace(&mut seen[i], true));
        if valid {
            // Keep pointing at the same item.
            let current = self.current_index();
            self.order = order;
            self.pos = current.and_then(|c| self.order.iter().position(|&i| i == c));
        }
        valid
    }

    pub fn set_repeat(&mut self, repeat: RepeatMode) {
        self.repeat = repeat;
    }

    pub fn repeat(&self) -> RepeatMode {
        self.repeat
    }

    pub fn items(&self) -> &[T] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn current_index(&self) -> Option<usize> {
        self.pos.map(|p| self.order[p])
    }

    pub fn current(&self) -> Option<&T> {
        self.current_index().map(|i| &self.items[i])
    }

    /// Make item `index` current.
    pub fn jump(&mut self, index: usize) -> Option<&T> {
        self.pos = self.order.iter().position(|&i| i == index);
        self.current()
    }

    fn next_pos(&self) -> Option<usize> {
        let len = self.order.len();
        if len == 0 {
            return None;
        }
        match self.pos {
            None => Some(0),
            Some(p) if p + 1 < len => Some(p + 1),
            Some(_) if self.repeat == RepeatMode::All => Some(0),
            Some(_) => None,
        }
    }

    /// What follows the current item (Repeat One is the engine's job:
    /// this returns the queue successor regardless).
    pub fn peek_next(&self) -> Option<&T> {
        self.next_pos().map(|p| &self.items[self.order[p]])
    }

    pub fn advance(&mut self) -> Option<&T> {
        self.pos = Some(self.next_pos()?);
        self.current()
    }

    pub fn previous(&mut self) -> Option<&T> {
        let len = self.order.len();
        self.pos = match self.pos? {
            0 if self.repeat == RepeatMode::All => Some(len - 1),
            0 => Some(0),
            p => Some(p - 1),
        };
        self.current()
    }
}

#[cfg(feature = "player")]
impl PlayQueue<crate::media::MediaDescriptor> {
    /// Keep the queue in step with the engine: when the engine reports
    /// that the queue's next item became current (a gapless or
    /// crossfaded transition), advance. Returns the item the engine
    /// should be given as its next one (send it with
    /// `AudioCommand::SetNext`), or `None` when nothing changed.
    pub fn follow(
        &mut self,
        event: &crate::engine::AudioEvent,
    ) -> Option<Option<crate::media::MediaDescriptor>> {
        let crate::engine::AudioEvent::TrackLoaded { track, .. } = event else {
            return None;
        };
        if self.current().is_some_and(|c| c.id == track.id) {
            return None;
        }
        if self.peek_next().is_some_and(|n| n.id == track.id) {
            self.advance();
        } else if let Some(i) = self.items.iter().position(|d| d.id == track.id) {
            self.jump(i);
        } else {
            return None;
        }
        Some(self.peek_next().cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_repeat_and_navigation() {
        let mut q = PlayQueue::new(vec!['a', 'b', 'c']);
        assert_eq!(q.peek_next(), Some(&'a'));
        assert_eq!(q.advance(), Some(&'a'));
        assert!(q.set_order(vec![2, 0, 1]));
        assert_eq!(q.current(), Some(&'a'));
        assert_eq!(q.peek_next(), Some(&'b'));
        q.advance();
        assert_eq!(q.peek_next(), None, "end of a non-repeating queue");
        q.set_repeat(RepeatMode::All);
        assert_eq!(q.peek_next(), Some(&'c'));
        assert_eq!(q.previous(), Some(&'a'));
        assert!(!q.set_order(vec![0, 0, 1]), "not a permutation");
    }
}
