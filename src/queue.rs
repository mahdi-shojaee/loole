use std::collections::VecDeque;

#[derive(Debug)]
pub struct Queue<T> {
    inner: VecDeque<(usize, T)>,
}

impl<T> Queue<T> {
    pub fn new() -> Self {
        Self {
            inner: VecDeque::new(),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: VecDeque::with_capacity(capacity),
        }
    }

    pub fn enqueue(&mut self, id: usize, value: T) {
        self.inner.push_back((id, value));
    }

    pub fn dequeue(&mut self) -> Option<(usize, T)> {
        self.inner.pop_front()
    }

    pub fn contains(&self, id: usize) -> bool {
        self.inner.iter().find(|(i, _)| *i == id).is_some()
    }

    pub fn remove(&mut self, id: usize) -> Option<(usize, T)> {
        self.inner.remove(self.index_by_id(id)?)
    }

    fn index_by_id(&self, id: usize) -> Option<usize> {
        self.inner
            .iter()
            .enumerate()
            .find(|(_, m)| m.0 == id)
            .map(|(index, _)| index)
    }
}
