use std::collections::{HashMap, VecDeque};
use std::hash::{BuildHasher, Hasher};

/// A hasher that hashes the key as its own identity. Ids in the pending
/// queues are sequential counters produced by `SharedState::get_next_id`, so
/// there is no adversarial control over keys and no collision/DoS risk. This
/// avoids the default SipHash hashing cost on every enqueue/dequeue/contains,
/// which is paid on the hot path of every parked send and async receive.
#[derive(Default, Clone)]
struct IdentityHasher {
    hash: u64,
}

impl Hasher for IdentityHasher {
    #[inline(always)]
    fn finish(&self) -> u64 {
        self.hash
    }

    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for chunk in &mut chunks {
            self.hash ^= u64::from_le_bytes(chunk.try_into().unwrap());
        }
        let rem = chunks.remainder();
        if !rem.is_empty() {
            let mut b = [0u8; 8];
            b[..rem.len()].copy_from_slice(rem);
            self.hash ^= u64::from_le_bytes(b);
        }
    }

    #[inline(always)]
    fn write_usize(&mut self, i: usize) {
        self.hash = i as u64;
    }

    #[inline(always)]
    fn write_u64(&mut self, i: u64) {
        self.hash = i;
    }
}

#[derive(Default, Clone)]
struct IdentityBuildHasher;

impl BuildHasher for IdentityBuildHasher {
    type Hasher = IdentityHasher;

    #[inline(always)]
    fn build_hasher(&self) -> IdentityHasher {
        IdentityHasher { hash: 0 }
    }
}

#[derive(Debug)]
pub struct Queue<T> {
    queue: VecDeque<usize>,
    map: HashMap<usize, T, IdentityBuildHasher>,
}

impl<T> Queue<T> {
    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            map: HashMap::with_hasher(IdentityBuildHasher),
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            queue: VecDeque::with_capacity(capacity),
            map: HashMap::with_capacity_and_hasher(capacity, IdentityBuildHasher),
        }
    }

    #[inline(always)]
    pub fn enqueue(&mut self, id: usize, value: T) {
        self.queue.push_back(id);
        self.map.insert(id, value);
    }

    #[inline(always)]
    pub fn enqueue_front(&mut self, id: usize, value: T) {
        self.queue.push_front(id);
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

    /// Mutable access to a queued value by id, without disturbing its queue
    /// position. Used to update a parked entry's waker in place.
    pub fn get_mut(&mut self, id: usize) -> Option<&mut T> {
        self.map.get_mut(&id)
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
    values: &'a HashMap<usize, T, IdentityBuildHasher>,
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
