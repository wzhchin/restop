use std::{iter::Chain, ops::Range};

#[derive(Debug)]
pub struct Ring<T> {
    len: usize,
    vec: Vec<T>,
    cursor: usize,
    pub name: String,
}

impl<T> Ring<T> {
    pub fn new(length: usize) -> Self {
        Self {
            len: length,
            vec: Vec::with_capacity(length),
            cursor: 0,
            name: "".to_owned(),
        }
    }

    pub fn name(self, name: &str) -> Self {
        Self {
            name: name.to_string(),
            ..self
        }
    }

    pub fn insert_at_first(&mut self, v: T) {
        if self.vec.len() < self.len {
            self.vec.insert(0, v);
        } else {
            self.cursor = (self.len.saturating_add(self.cursor).saturating_sub(1)) % self.len;

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
            chain: (self.cursor..self.vec.len()).chain(0..self.cursor),
            len: self.vec.len(),
        }
    }
}

pub struct IterRing<'r, T> {
    vec: &'r Vec<T>,
    chain: Chain<Range<usize>, Range<usize>>,
    len: usize,
}

impl<'r, T> IterRing<'r, T> {
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl<'r, T> Iterator for IterRing<'r, T> {
    type Item = &'r T;

    fn next(&mut self) -> Option<Self::Item> {
        match self.chain.next() {
            Some(id) => self.vec.get(id),
            None => None,
        }
    }
}
