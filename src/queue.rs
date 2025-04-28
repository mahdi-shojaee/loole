use std::collections::{HashMap, VecDeque};

#[derive(Debug)]
pub struct Queue<T> {
    queue: VecDeque<usize>,
    map: HashMap<usize, T>,
}

impl<T> Queue<T> {
    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            map: HashMap::new(),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            queue: VecDeque::with_capacity(capacity),
            map: HashMap::with_capacity(capacity),
        }
    }

    #[inline(always)]
    pub fn enqueue(&mut self, id: usize, value: T) {
        self.queue.push_back(id);
        self.map.insert(id, value);
    }

    #[inline(always)]
    pub fn dequeue(&mut self) -> Option<(usize, T)> {
        let id = self.queue.pop_front()?;
        let value = self.map.remove(&id)?;
        Some((id, value))
    }

    pub fn contains(&self, id: usize) -> bool {
        self.map.contains_key(&id)
    }

    pub fn get(&self, id: usize) -> Option<&T> {
        self.map.get(&id)
    }

    pub fn remove(&mut self, id: usize) -> Option<(usize, T)> {
        if let Some(value) = self.map.remove(&id) {
            if let Some(index) = self.queue.iter().position(|&x| x == id) {
                self.queue.remove(index);
            }
            Some((id, value))
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    #[inline(always)]
    pub fn iter(&self) -> Iter<'_, T> {
        Iter {
            keys: self.queue.iter(),
            values: &self.map,
        }
    }
}

impl<T> Default for Queue<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Iter<'a, T> {
    keys: std::collections::vec_deque::Iter<'a, usize>,
    values: &'a HashMap<usize, T>,
}

impl<'a, T> Iterator for Iter<'a, T> {
    type Item = (usize, &'a T);

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        let id = self.keys.next()?;
        self.values.get(id).map(|value| (*id, value))
    }
}

impl<'a, T> IntoIterator for &'a Queue<T> {
    type Item = (usize, &'a T);
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
        assert_eq!(collected, vec![(1, &10), (2, &20), (3, &30)]);
    }

    #[test]
    fn test_into_iter() {
        let mut queue = Queue::new();
        queue.enqueue(1, 10);
        queue.enqueue(2, 20);
        queue.enqueue(3, 30);

        let collected: Vec<_> = queue.into_iter().collect();
        assert_eq!(collected, vec![(1, &10), (2, &20), (3, &30)]);
    }
}
