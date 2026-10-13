//! A graph's history (SPEC §8.3, §4.4 item 7): the newest raw values, one per plot column, kept
//! in the sensor instance and never persisted.

use std::collections::VecDeque;

#[derive(Debug, Clone, PartialEq)]
pub struct History {
    values: VecDeque<f32>,
    capacity: usize,
}

impl History {
    pub fn new(capacity: usize) -> Self {
        Self {
            values: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Adds the newest value, dropping the oldest when full.
    pub fn push(&mut self, value: f32) {
        if self.capacity == 0 {
            return;
        }
        if self.values.len() == self.capacity {
            self.values.pop_front();
        }
        self.values.push_back(value);
    }

    /// Changes the capacity, keeping the newest values.
    pub fn resize(&mut self, capacity: usize) {
        while self.values.len() > capacity {
            self.values.pop_front();
        }
        self.capacity = capacity;
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Oldest first.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = f32> + '_ {
        self.values.iter().copied()
    }

    pub fn latest(&self) -> Option<f32> {
        self.values.back().copied()
    }

    /// The largest value, e.g. to scale a network graph (SPEC §9.13); 0 when empty.
    pub fn max(&self) -> f32 {
        self.values.iter().copied().fold(0.0, f32::max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_newest_values() {
        let mut h = History::new(3);
        assert!(h.is_empty());
        for v in [1.0, 2.0, 3.0, 4.0] {
            h.push(v);
        }
        assert_eq!(h.iter().collect::<Vec<_>>(), [2.0, 3.0, 4.0]);
        assert_eq!(h.latest(), Some(4.0));
        assert_eq!(h.max(), 4.0);
        h.resize(2);
        assert_eq!(h.iter().collect::<Vec<_>>(), [3.0, 4.0]);
        h.resize(5);
        h.push(5.0);
        assert_eq!(h.iter().collect::<Vec<_>>(), [3.0, 4.0, 5.0]);
        assert_eq!((h.len(), h.capacity()), (3, 5));
        let mut none = History::new(0);
        none.push(1.0);
        assert!(none.is_empty());
    }
}
