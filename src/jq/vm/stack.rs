//! Persistent, chunked VM stacks. Checkpoints share a head in O(1).
//! Unique chunks reuse their allocation for push/pop; shared chunks keep their
//! immutable prefix, and a new branch appends a chunk without copying history.
use std::rc::Rc;

#[derive(Debug)]
struct Node<T, const N: usize> {
    values: [Option<T>; N],
    filled: usize,
    previous: Stack<T, N>,
}

impl<T, const N: usize> Node<T, N> {
    fn truncate(&mut self, len: usize) {
        while self.filled > len {
            self.filled -= 1;
            self.values[self.filled] = None;
        }
    }
}

#[derive(Debug)]
pub struct Stack<T, const N: usize = 8> {
    head: Option<Rc<Node<T, N>>>,
    used: usize,
    len: usize,
}

impl<T, const N: usize> Default for Stack<T, N> {
    fn default() -> Self {
        Self {
            head: None,
            used: 0,
            len: 0,
        }
    }
}

impl<T, const N: usize> Clone for Stack<T, N> {
    fn clone(&self) -> Self {
        // Do not share an empty reusable allocation with a checkpoint.
        if self.len == 0 {
            return Self::default();
        }
        Self {
            head: self.head.clone(),
            used: self.used,
            len: self.len,
        }
    }
}

impl<T, const N: usize> Stack<T, N> {
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Reuse a uniquely owned chunk when restoring an empty checkpoint.
    /// Clear both its values and history so scratch space retains no operands.
    pub fn restore(&mut self, saved: Self) {
        if saved.len == 0
            && let Some(node) = self.head.as_mut().and_then(Rc::get_mut)
        {
            node.truncate(0);
            node.previous = Self::default();
            self.used = 0;
            self.len = 0;
        } else {
            *self = saved;
        }
    }

    pub fn push(&mut self, value: T) {
        const {
            assert!(N > 0);
        }
        if self.used < N
            && let Some(node) = self.head.as_mut().and_then(Rc::get_mut)
        {
            node.truncate(self.used);
            node.values[self.used] = Some(value);
            node.filled += 1;
            self.used += 1;
            self.len += 1;
            return;
        }
        let previous = if self.len == 0 {
            Self::default()
        } else {
            std::mem::take(self)
        };
        self.len = previous.len + 1;
        self.used = 1;
        let mut values = std::array::from_fn(|_| None);
        values[0] = Some(value);
        self.head = Some(Rc::new(Node {
            values,
            filled: 1,
            previous,
        }));
    }

    pub fn last(&self) -> Option<&T> {
        self.used
            .checked_sub(1)
            .map(|i| self.head.as_ref().unwrap().values[i].as_ref().unwrap())
    }

    /// Iterate newest to oldest, matching recovery/label lookup order.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        std::iter::successors(Some(self), |stack| stack.head.as_ref().map(|n| &n.previous))
            .flat_map(|stack| {
                stack.head.as_ref().into_iter().flat_map(move |node| {
                    node.values[..stack.used]
                        .iter()
                        .rev()
                        .map(|value| value.as_ref().unwrap())
                })
            })
    }

    fn retreat(&mut self) {
        self.len -= 1;
        self.used -= 1;
        if self.used == 0 && self.len > 0 {
            let previous = if let Some(node) = self.head.as_mut().and_then(Rc::get_mut) {
                std::mem::take(&mut node.previous)
            } else {
                self.head.as_ref().unwrap().previous.clone()
            };
            *self = previous;
        }
    }

    pub fn truncate(&mut self, len: usize) {
        while self.len > len {
            if let Some(node) = self.head.as_mut().and_then(Rc::get_mut) {
                node.truncate(self.used - 1);
            }
            self.retreat();
        }
    }
}

impl<T: Clone, const N: usize> Stack<T, N> {
    pub fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        let value = if let Some(node) = self.head.as_mut().and_then(Rc::get_mut) {
            node.truncate(self.used);
            node.filled -= 1;
            node.values[self.used - 1].take().unwrap()
        } else {
            self.last().unwrap().clone()
        };
        self.retreat();
        Some(value)
    }

    pub fn split_off(&mut self, at: usize) -> Vec<T> {
        assert!(at <= self.len);
        let mut values = Vec::with_capacity(self.len - at);
        while self.len > at {
            values.push(self.pop().unwrap());
        }
        values.reverse();
        values
    }

    pub fn to_vec(&self) -> Vec<T> {
        let mut values: Vec<_> = self.iter().cloned().collect();
        values.reverse();
        values
    }

    /// Discard the matching item without cloning it from a shared checkpoint.
    pub fn remove_first(&mut self, predicate: impl Fn(&T) -> bool) -> Option<()> {
        let depth = self.iter().position(predicate)?;
        let above = self.split_off(self.len - depth);
        self.truncate(self.len - 1);
        for value in above {
            self.push(value);
        }
        Some(())
    }
}

impl<T, const N: usize> FromIterator<T> for Stack<T, N> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let mut stack = Self::default();
        for value in iter {
            stack.push(value);
        }
        stack
    }
}

impl<T, const N: usize> std::ops::Index<usize> for Stack<T, N> {
    type Output = T;
    fn index(&self, index: usize) -> &T {
        assert!(index < self.len, "stack index");
        let mut stack = self;
        loop {
            let node = stack.head.as_ref().unwrap();
            if index >= node.previous.len {
                return node.values[index - node.previous.len].as_ref().unwrap();
            }
            stack = &node.previous;
        }
    }
}

impl<T, const N: usize> Drop for Stack<T, N> {
    fn drop(&mut self) {
        // A long unique chain must not recurse through Rc destructors.
        while let Some(head) = self.head.take() {
            match Rc::try_unwrap(head) {
                Ok(mut node) => self.head = node.previous.head.take(),
                Err(_) => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Stack;
    use std::{cell::Cell, rc::Rc};

    #[derive(Debug)]
    struct Counted(Rc<Cell<usize>>);
    impl Clone for Counted {
        fn clone(&self) -> Self {
            self.0.set(self.0.get() + 1);
            Self(self.0.clone())
        }
    }

    #[test]
    fn snapshots_share_history_and_unique_pops_move_values() {
        let clones = Rc::new(Cell::new(0));
        let mut stack: Stack<_> = (0..100_000).map(|_| Counted(clones.clone())).collect();
        let saved = stack.clone();
        assert_eq!(clones.get(), 0);
        stack.pop();
        assert_eq!(clones.get(), 1);
        drop(saved);
        stack.pop();
        assert_eq!(clones.get(), 1);
        // Also exercises iterative destruction of a long unique suffix.
        drop(stack);
    }

    #[test]
    fn branches_and_middle_removal_preserve_saved_history() {
        let mut stack: Stack<_> = (0..6).collect();
        let saved = stack.clone();
        assert_eq!(stack.remove_first(|x| *x == 3), Some(()));
        assert_eq!(stack.to_vec(), [0, 1, 2, 4, 5]);
        stack.truncate(2);
        stack.push(9);
        assert_eq!(stack.to_vec(), [0, 1, 9]);
        assert_eq!(saved.to_vec(), [0, 1, 2, 3, 4, 5]);
        assert_eq!(stack[1], 1);
        assert_eq!(stack.split_off(1), [1, 9]);
        assert_eq!(stack.to_vec(), [0]);
    }

    #[test]
    fn discarding_a_shared_top_does_not_clone_it() {
        let clones = Rc::new(Cell::new(0));
        let mut stack: Stack<_, 1> = [Counted(clones.clone())].into_iter().collect();
        let saved = stack.clone();
        assert_eq!(stack.remove_first(|_| true), Some(()));
        assert_eq!(clones.get(), 0);
        assert_eq!(saved.len(), 1);
    }

    #[test]
    fn restoring_empty_reuses_only_unique_chunks_and_drops_stale_values() {
        let payload = Rc::new(Cell::new(0));
        let mut stack: Stack<_, 1> = [payload.clone()].into_iter().collect();
        let saved = stack.clone();
        stack.truncate(0);
        let head = Rc::as_ptr(stack.head.as_ref().unwrap());
        drop(saved);
        stack.restore(Stack::default());
        assert_eq!(Rc::strong_count(&payload), 1);
        stack.push(payload.clone());
        assert_eq!(Rc::as_ptr(stack.head.as_ref().unwrap()), head);

        let saved = stack.clone();
        stack.truncate(0);
        stack.restore(Stack::default());
        assert!(stack.head.is_none());
        assert_eq!(saved.len(), 1);
        assert!(Rc::ptr_eq(saved.last().unwrap(), &payload));
    }

    #[test]
    fn restoring_empty_reuses_a_nonempty_chunk_and_releases_history() {
        let payload = Rc::new(());
        let mut stack: Stack<_, 2> = (0..5).map(|_| payload.clone()).collect();
        let head = Rc::as_ptr(stack.head.as_ref().unwrap());
        stack.restore(Stack::default());
        assert_eq!(Rc::strong_count(&payload), 1);
        assert_eq!(stack.len(), 0);
        assert_eq!(stack.last(), None);
        assert_eq!(stack.iter().count(), 0);
        // Repeated nonempty restores must keep the same allocation.
        for _ in 0..100 {
            stack.push(payload.clone());
            assert_eq!(Rc::as_ptr(stack.head.as_ref().unwrap()), head);
            stack.restore(Stack::default());
            assert_eq!(Rc::strong_count(&payload), 1);
        }
    }

    #[test]
    fn restoring_empty_does_not_clear_a_shared_nonempty_chunk() {
        let mut stack: Stack<_, 2> = (0..5).collect();
        let saved = stack.clone();
        stack.restore(Stack::default());
        stack.push(99);
        assert_eq!(stack.to_vec(), [99]);
        assert_eq!(saved.to_vec(), [0, 1, 2, 3, 4]);
        stack.restore(saved);
        assert_eq!(stack.to_vec(), [0, 1, 2, 3, 4]);
    }

    #[test]
    fn partial_chunks_can_branch_restore_and_become_unique_again() {
        let mut stack: Stack<_> = (0..100).collect();
        let saved = stack.clone();
        stack.truncate(35);
        let partial = stack.clone();
        stack.push(200);
        stack.push(201);
        assert_eq!(stack.split_off(35), [200, 201]);
        assert_eq!(stack.to_vec(), (0..35).collect::<Vec<_>>());
        drop(saved);
        stack = partial;
        stack.truncate(33);
        stack.push(300);
        assert_eq!(stack.to_vec(), (0..33).chain([300]).collect::<Vec<_>>());
        stack.truncate(0);
        stack.push(400);
        assert_eq!(stack.pop(), Some(400));
        assert_eq!(stack.pop(), None);
        assert_eq!(stack.last(), None);
    }
}
