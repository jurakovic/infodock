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

    /// Oldest first.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = f32> + '_ {
        self.values.iter().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_newest_values() {
        let mut h = History::new(3);
        assert_eq!(h.iter().count(), 0);
        for v in [1.0, 2.0, 3.0, 4.0] {
            h.push(v);
        }
        assert_eq!(h.iter().collect::<Vec<_>>(), [2.0, 3.0, 4.0]);
        let mut none = History::new(0);
        none.push(1.0);
        assert_eq!(none.iter().count(), 0);
    }
}
