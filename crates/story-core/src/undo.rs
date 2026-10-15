use std::collections::VecDeque;

use crate::{MAX_UNDO_CAPACITY, StoryError, StoryState};

/// Bounded ring of previous states for undo.
///
/// The session layer pushes the current state before each accepted
/// [`crate::step`] and pops to undo. When full, the oldest entry is evicted, so
/// memory is bounded by `capacity` states. The ring is session memory only:
/// durable save slots are a later phase with their own ADR.
#[derive(Debug, Clone)]
pub struct UndoRing {
    capacity: usize,
    entries: VecDeque<StoryState>,
}

impl UndoRing {
    /// Creates a ring holding `1..=MAX_UNDO_CAPACITY` states.
    pub fn new(capacity: usize) -> Result<Self, StoryError> {
        if capacity == 0 || capacity > MAX_UNDO_CAPACITY {
            return Err(StoryError::InvalidUndoCapacity {
                max: MAX_UNDO_CAPACITY,
            });
        }
        Ok(Self {
            capacity,
            entries: VecDeque::with_capacity(capacity),
        })
    }

    /// Records `state`, evicting and returning the oldest entry when full.
    pub fn push(&mut self, state: StoryState) -> Option<StoryState> {
        let evicted = if self.entries.len() >= self.capacity {
            self.entries.pop_front()
        } else {
            None
        };
        self.entries.push_back(state);
        evicted
    }

    /// Removes and returns the most recent state.
    pub fn pop(&mut self) -> Option<StoryState> {
        self.entries.pop_back()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}
