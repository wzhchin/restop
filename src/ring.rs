/// Default history capacity. Graph uses 2 samples per column (braille),
/// so 256 covers ~128 columns of history without excess memory.
pub const DEFAULT_HISTORY_LEN: usize = 256;

/// Per-thread / secondary graph history (narrower widgets).
pub const SHORT_HISTORY_LEN: usize = 128;

#[derive(Debug)]
pub struct Ring<T> {
    len: usize,
    vec: Vec<T>,
    /// Index of the newest sample once the buffer is non-empty.
    cursor: usize,
    pub name: String,
}

impl<T> Ring<T> {
    pub fn new(length: usize) -> Self {
        Self {
            len: length.max(1),
            vec: Vec::with_capacity(length.max(1)),
            cursor: 0,
            name: String::new(),
        }
    }

    pub fn name(self, name: &str) -> Self {
        Self {
            name: name.to_string(),
            ..self
        }
    }

    /// Push a new sample (O(1)). Newest sample becomes the head of
    /// [`new_to_old_iter`].
    pub fn insert_at_first(&mut self, v: T) {
        if self.vec.len() < self.len {
            self.vec.push(v);
            self.cursor = self.vec.len() - 1;
        } else {
            self.cursor = (self.cursor + 1) % self.len;
            self.vec[self.cursor] = v;
        }
    }

    pub fn newest(&self) -> Option<&T> {
        if self.vec.is_empty() {
            None
        } else {
            Some(&self.vec[self.cursor])
        }
    }

    pub fn new_to_old_iter(&self) -> IterRing<'_, T> {
        IterRing {
            vec: &self.vec,
            index: self.cursor,
            remaining: self.vec.len(),
        }
    }
}

pub struct IterRing<'r, T> {
    vec: &'r Vec<T>,
    index: usize,
    remaining: usize,
}

impl<'r, T> IterRing<'r, T> {
    pub fn len(&self) -> usize {
        self.remaining
    }

    pub fn is_empty(&self) -> bool {
        self.remaining == 0
    }
}

impl<'r, T> Iterator for IterRing<'r, T> {
    type Item = &'r T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let item = self.vec.get(self.index)?;
        self.remaining -= 1;
        if self.remaining > 0 {
            self.index = if self.index == 0 {
                self.vec.len() - 1
            } else {
                self.index - 1
            };
        }
        Some(item)
    }
}

#[cfg(test)]
mod tests {
    use super::Ring;

    fn collected(ring: &Ring<i32>) -> Vec<i32> {
        ring.new_to_old_iter().copied().collect()
    }

    #[test]
    fn filling_yields_newest_to_oldest() {
        let mut ring = Ring::new(4);
        assert!(collected(&ring).is_empty());
        assert_eq!(ring.newest(), None);

        ring.insert_at_first(1);
        assert_eq!(collected(&ring), vec![1]);
        ring.insert_at_first(2);
        assert_eq!(collected(&ring), vec![2, 1]);
        ring.insert_at_first(3);
        assert_eq!(collected(&ring), vec![3, 2, 1]);
        assert_eq!(ring.newest().copied(), Some(3));
    }

    #[test]
    fn wrap_keeps_newest_first_so_graphs_scroll() {
        let mut ring = Ring::new(3);
        for v in [1, 2, 3] {
            ring.insert_at_first(v);
        }
        assert_eq!(collected(&ring), vec![3, 2, 1]);

        ring.insert_at_first(4);
        assert_eq!(collected(&ring), vec![4, 3, 2]);
        ring.insert_at_first(5);
        assert_eq!(collected(&ring), vec![5, 4, 3]);
        assert_eq!(ring.newest().copied(), Some(5));
    }
}
