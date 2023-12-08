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
        self.inner.iter().any(|(i, _)| *i == id)
    }

    pub fn remove(&mut self, id: usize) -> Option<(usize, T)> {
        self.inner.remove(self.index_by_id(id)?)
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut (usize, T)> {
        self.inner.get_mut(index)
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn iter(&self) -> Iter<'_, T> {
        Iter {
            inner: self.inner.iter(),
        }
    }

    fn index_by_id(&self, id: usize) -> Option<usize> {
        self.inner
            .iter()
            .enumerate()
            .find(|(_, m)| m.0 == id)
            .map(|(index, _)| index)
    }
}

pub struct Iter<'a, T> {
    inner: std::collections::vec_deque::Iter<'a, (usize, T)>,
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a (usize, T);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

impl<'a, T> IntoIterator for &'a Queue<T> {
    type Item = &'a (usize, T);
    type IntoIter = Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
