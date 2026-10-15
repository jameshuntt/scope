//! Generational task identity + a slab that stores tasks by it.
//!
//! A [`TaskId`] names a `slot` plus a `generation`; freeing a slot bumps its
//! generation, so a stale id (a wake or join subscription that outlived its
//! task) resolves to `None` *by construction*. That single property removes a
//! whole class of "is this deadline still valid?" checks.

use std::fmt;

/// Stable, reuse-safe handle to a task inside a [`crate::Scope`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TaskId {
    /// Index into the slab.
    pub slot: u32,
    /// Reuse counter of the slot; a stale id carries an older generation.
    pub generation: u32,
}

impl TaskId {
    /// An id from its parts. Ids normally come from [`TaskSlab::insert`].
    pub const fn new(slot: u32, generation: u32) -> Self {
        Self { slot, generation }
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}.{}", self.slot, self.generation)
    }
}

/// `repr(C)` with `generation` first: a lookup checks the generation and then
/// reads the start of the value, so both sit at the front of the slot (the
/// scheduler's task entry puts its per-round fields first to match).
#[repr(C)]
struct Slot<T> {
    generation: u32,
    value: Option<T>,
}

/// A generational arena. `O(1)` insert/get/remove, ids never alias across reuse.
pub struct TaskSlab<T> {
    slots: Vec<Slot<T>>,
    free: Vec<u32>,
    len: usize,
}

impl<T> Default for TaskSlab<T> {
    fn default() -> Self {
        Self { slots: Vec::new(), free: Vec::new(), len: 0 }
    }
}

impl<T> TaskSlab<T> {
    /// An empty slab.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of live entries.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether no entries are live.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Store `value`, reusing a freed slot when one exists, and return its id.
    pub fn insert(&mut self, value: T) -> TaskId {
        self.len += 1;
        if let Some(slot_idx) = self.free.pop() {
            let slot = &mut self.slots[slot_idx as usize];
            slot.value = Some(value);
            TaskId::new(slot_idx, slot.generation)
        } else {
            let slot_idx = self.slots.len() as u32;
            self.slots.push(Slot { generation: 0, value: Some(value) });
            TaskId::new(slot_idx, 0)
        }
    }

    /// Whether `id` names a live entry: right slot and right generation.
    pub fn contains(&self, id: TaskId) -> bool {
        self.get(id).is_some()
    }

    /// The entry `id` names, or `None` once it was removed or the slot reused.
    pub fn get(&self, id: TaskId) -> Option<&T> {
        let slot = self.slots.get(id.slot as usize)?;
        if slot.generation == id.generation {
            slot.value.as_ref()
        } else {
            None
        }
    }

    /// Mutable [`TaskSlab::get`].
    pub fn get_mut(&mut self, id: TaskId) -> Option<&mut T> {
        let slot = self.slots.get_mut(id.slot as usize)?;
        if slot.generation == id.generation {
            slot.value.as_mut()
        } else {
            None
        }
    }

    /// Take the entry out and bump the slot generation, so `id` is dead from now on.
    pub fn remove(&mut self, id: TaskId) -> Option<T> {
        let slot = self.slots.get_mut(id.slot as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        let value = slot.value.take()?;
        slot.generation = slot.generation.wrapping_add(1);
        self.free.push(id.slot);
        self.len -= 1;
        Some(value)
    }

    /// Live ids in slot order (deterministic — the backbone of reproducible
    /// scheduling).
    pub fn live_ids(&self) -> Vec<TaskId> {
        let mut out = Vec::with_capacity(self.len);
        out.extend(self.entries().map(|(id, _)| id));
        out
    }

    /// Live entries with their ids, in slot order. Stops at the last live
    /// entry instead of walking the freed slots after it.
    pub(crate) fn entries(&self) -> impl Iterator<Item = (TaskId, &T)> + '_ {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(idx, slot)| {
                let value = slot.value.as_ref()?;
                Some((TaskId::new(idx as u32, slot.generation), value))
            })
            .take(self.len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_id_after_reuse_resolves_to_none() {
        let mut slab: TaskSlab<&str> = TaskSlab::new();
        let a = slab.insert("a");
        assert_eq!(slab.remove(a), Some("a"));
        // Reusing the freed slot mints a new generation.
        let b = slab.insert("b");
        assert_eq!(a.slot, b.slot);
        assert_ne!(a.generation, b.generation);
        // The stale id no longer resolves — the whole point.
        assert!(slab.get(a).is_none());
        assert_eq!(slab.get(b), Some(&"b"));
    }

    #[test]
    fn len_tracks_live_entries() {
        let mut slab: TaskSlab<u32> = TaskSlab::new();
        assert!(slab.is_empty());
        let a = slab.insert(1);
        let _b = slab.insert(2);
        assert_eq!(slab.len(), 2);
        slab.remove(a);
        assert_eq!(slab.len(), 1);
        assert_eq!(slab.live_ids().len(), 1);
    }
}
