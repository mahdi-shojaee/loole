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

    #[inline(always)]
    pub fn enqueue(&mut self, id: usize, value: T) {
        self.inner.push_back((id, value));
    }

    #[inline(always)]
    pub fn dequeue(&mut self) -> Option<(usize, T)> {
        self.inner.pop_front()
    }

    pub fn contains(&self, id: usize) -> bool {
        self.inner.iter().any(|&(i, _)| i == id)
    }

    pub fn remove(&mut self, id: usize) -> Option<(usize, T)> {
        self.inner.remove(self.index_by_id(id)?)
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    #[inline(always)]
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

impl<T> Default for Queue<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Iter<'a, T> {
    inner: std::collections::vec_deque::Iter<'a, (usize, T)>,
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = &'a (usize, T);

    #[inline(always)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_queue() {
        let queue: Queue<i32> = Queue::new();
        assert_eq!(queue.len(), 0);
    }

    #[test]
    fn test_with_capacity() {
        let queue: Queue<i32> = Queue::with_capacity(10);
        assert_eq!(queue.len(), 0);
    }

    #[test]
    fn test_enqueue_dequeue() {
        let mut queue = Queue::new();
        queue.enqueue(1, 10);
        queue.enqueue(2, 20);
        queue.enqueue(3, 30);

        assert_eq!(queue.len(), 3);
        assert_eq!(queue.dequeue(), Some((1, 10)));
        assert_eq!(queue.dequeue(), Some((2, 20)));
        assert_eq!(queue.dequeue(), Some((3, 30)));
        assert_eq!(queue.dequeue(), None);
        assert_eq!(queue.len(), 0);
    }

    #[test]
    fn test_contains() {
        let mut queue = Queue::new();
        queue.enqueue(1, "one");
        queue.enqueue(2, "two");
        queue.enqueue(3, "three");

        assert!(queue.contains(1));
        assert!(queue.contains(2));
        assert!(queue.contains(3));
        assert!(!queue.contains(4));
    }

    #[test]
    fn test_remove() {
        let mut queue = Queue::new();
        queue.enqueue(1, "one");
        queue.enqueue(2, "two");
        queue.enqueue(3, "three");

        assert_eq!(queue.remove(2), Some((2, "two")));
        assert_eq!(queue.len(), 2);
        assert!(!queue.contains(2));

        assert_eq!(queue.remove(4), None);
    }

    #[test]
    fn test_iter() {
        let mut queue = Queue::new();
        queue.enqueue(1, 10);
        queue.enqueue(2, 20);
        queue.enqueue(3, 30);

        let collected: Vec<_> = queue.iter().collect();
        assert_eq!(collected, vec![&(1, 10), &(2, 20), &(3, 30)]);
    }

    #[test]
    fn test_into_iter() {
        let mut queue = Queue::new();
        queue.enqueue(1, 10);
        queue.enqueue(2, 20);
        queue.enqueue(3, 30);

        let collected: Vec<_> = queue.into_iter().collect();
        assert_eq!(collected, vec![&(1, 10), &(2, 20), &(3, 30)]);
    }
}
