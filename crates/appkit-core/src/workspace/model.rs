//! Generic, headless workspace model.
//!
//! `WorkspaceModel<T>` tracks tab ordering, active selection, pinning and
//! navigation history using stable `TabId`s. It does not know the concrete
//! document or runtime type stored in each entry.

use std::collections::HashSet;

use crate::navigator::NavEffect;
use crate::workspace::types::{TabId, TabIdAllocator};

/// A single tab in the generic workspace model.
pub struct WorkspaceEntry<T> {
    pub id: TabId,
    pub value: T,
    pub suggested_file_name: Option<String>,
}

impl<T> WorkspaceEntry<T> {
    pub fn new(id: TabId, value: T, suggested_file_name: Option<String>) -> Self {
        Self { id, value, suggested_file_name }
    }
}

/// Effect report produced by `WorkspaceModel::close_by_id`.
pub struct CloseEffect {
    /// The index the closed tab occupied before removal.
    pub removed_index: usize,
    /// The index of the tab that became active after the close, if any.
    pub new_active_index: usize,
}

/// Headless container for tab ordering, active selection, pinning and
/// navigation history.
pub struct WorkspaceModel<T> {
    entries: Vec<WorkspaceEntry<T>>,
    active_id: Option<TabId>,
    pinned_ids: HashSet<TabId>,
    back_history: Vec<TabId>,
    forward_history: Vec<TabId>,
    id_allocator: TabIdAllocator,
}

impl<T> WorkspaceModel<T> {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            active_id: None,
            pinned_ids: HashSet::new(),
            back_history: Vec::new(),
            forward_history: Vec::new(),
            id_allocator: TabIdAllocator::new(),
        }
    }

    pub fn allocate_id(&mut self) -> TabId {
        self.id_allocator.allocate()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn entries(&self) -> &[WorkspaceEntry<T>] {
        &self.entries
    }

    pub fn entries_mut(&mut self) -> &mut [WorkspaceEntry<T>] {
        &mut self.entries
    }

    pub fn entry(&self, index: usize) -> Option<&WorkspaceEntry<T>> {
        self.entries.get(index)
    }

    pub fn entry_mut(&mut self, index: usize) -> Option<&mut WorkspaceEntry<T>> {
        self.entries.get_mut(index)
    }

    pub fn entry_by_id(&self, id: TabId) -> Option<&WorkspaceEntry<T>> {
        self.entries.iter().find(|e| e.id == id)
    }

    pub fn entry_by_id_mut(&mut self, id: TabId) -> Option<&mut WorkspaceEntry<T>> {
        self.entries.iter_mut().find(|e| e.id == id)
    }

    pub fn active_id(&self) -> Option<TabId> {
        self.active_id
    }

    pub fn active_index(&self) -> usize {
        self.active_id.and_then(|id| self.index_of(id)).unwrap_or(0)
    }

    pub fn active_entry(&self) -> Option<&WorkspaceEntry<T>> {
        self.active_id.and_then(|id| self.entry_by_id(id))
    }

    pub fn active_entry_mut(&mut self) -> Option<&mut WorkspaceEntry<T>> {
        let id = self.active_id?;
        self.entry_by_id_mut(id)
    }

    pub fn index_of(&self, id: TabId) -> Option<usize> {
        self.entries.iter().position(|e| e.id == id)
    }

    pub fn id_at(&self, index: usize) -> Option<TabId> {
        self.entries.get(index).map(|e| e.id)
    }

    pub fn pinned_ids(&self) -> &HashSet<TabId> {
        &self.pinned_ids
    }

    pub fn is_pinned(&self, id: TabId) -> bool {
        self.pinned_ids.contains(&id)
    }

    pub fn back_history(&self) -> &[TabId] {
        &self.back_history
    }

    pub fn forward_history(&self) -> &[TabId] {
        &self.forward_history
    }

    /// Test-only helpers to inject arbitrary navigation history.
    pub fn set_back_history(&mut self, ids: Vec<TabId>) {
        self.back_history = ids;
    }

    pub fn set_forward_history(&mut self, ids: Vec<TabId>) {
        self.forward_history = ids;
    }

    /// Append an already-built entry and return its stable ID. If the workspace
    /// was empty, the new entry becomes active.
    pub fn push_entry(&mut self, entry: WorkspaceEntry<T>) -> TabId {
        let id = entry.id;
        self.entries.push(entry);
        if self.active_id.is_none() {
            self.active_id = Some(id);
        }
        id
    }

    /// Append a new entry and return its stable ID. If the workspace was empty,
    /// the new entry becomes active.
    pub fn push(&mut self, value: T, suggested_file_name: Option<String>) -> TabId {
        let id = self.allocate_id();
        self.push_entry(WorkspaceEntry::new(id, value, suggested_file_name));
        id
    }

    /// Insert a new entry at `index` and return its stable ID. If the workspace
    /// was empty, the new entry becomes active.
    pub fn insert_at(
        &mut self,
        index: usize,
        value: T,
        suggested_file_name: Option<String>,
    ) -> TabId {
        let id = self.allocate_id();
        let clamped = index.min(self.entries.len());
        self.entries.insert(clamped, WorkspaceEntry::new(id, value, suggested_file_name));
        if self.active_id.is_none() {
            self.active_id = Some(id);
        }
        id
    }

    /// Switch the active tab to `id`. Returns `ActiveChanged` if the active tab
    /// actually changed.
    pub fn switch_to(&mut self, id: TabId) -> NavEffect {
        if self.index_of(id).is_none() {
            return NavEffect::None;
        }
        if self.active_id == Some(id) {
            return NavEffect::None;
        }
        self.record_nav_step();
        self.active_id = Some(id);
        self.forward_history.clear();
        NavEffect::ActiveChanged
    }

    /// Record the current active tab into back-history before a navigation.
    pub fn record_nav_step(&mut self) {
        if let Some(id) = self.active_id {
            self.back_history.push(id);
            const MAX_NAV_HISTORY: usize = 200;
            if self.back_history.len() > MAX_NAV_HISTORY {
                self.back_history.drain(0..self.back_history.len() - MAX_NAV_HISTORY);
            }
        }
        self.forward_history.clear();
    }

    /// Set the active tab by ID without recording a navigation step. Returns
    /// `false` if the ID does not exist.
    pub fn set_active_id(&mut self, id: TabId) -> bool {
        if self.index_of(id).is_none() {
            return false;
        }
        self.active_id = Some(id);
        true
    }

    pub fn go_back(&mut self) -> NavEffect {
        while let Some(id) = self.back_history.pop() {
            if self.index_of(id).is_some() {
                if let Some(current) = self.active_id {
                    self.forward_history.push(current);
                }
                self.active_id = Some(id);
                return NavEffect::ActiveChanged;
            }
        }
        NavEffect::None
    }

    pub fn go_forward(&mut self) -> NavEffect {
        while let Some(id) = self.forward_history.pop() {
            if self.index_of(id).is_some() {
                if let Some(current) = self.active_id {
                    self.back_history.push(current);
                }
                self.active_id = Some(id);
                return NavEffect::ActiveChanged;
            }
        }
        NavEffect::None
    }

    /// Close the tab with `id`. Returns the index it occupied and the new active
    /// index, or `None` if the ID was not found.
    pub fn close_by_id(&mut self, id: TabId) -> Option<CloseEffect> {
        let removed_index = self.index_of(id)?;
        let was_active = self.active_id == Some(id);

        self.entries.remove(removed_index);

        self.pinned_ids.remove(&id);
        self.back_history.retain(|&i| i != id);
        self.forward_history.retain(|&i| i != id);

        let new_active_id = if was_active {
            self.choose_new_active_after_close(removed_index)
        } else {
            self.active_id
        };
        self.active_id = new_active_id;

        let new_active_index = new_active_id.and_then(|aid| self.index_of(aid)).unwrap_or(0);

        Some(CloseEffect { removed_index, new_active_index })
    }

    fn choose_new_active_after_close(&self, removed_index: usize) -> Option<TabId> {
        if self.entries.is_empty() {
            return None;
        }
        let target = removed_index.min(self.entries.len() - 1);
        Some(self.entries[target].id)
    }

    /// Pin the tab with `id` if it exists. Idempotent: repeated calls keep the tab pinned.
    /// Returns the pinned state after the call (`false` only if the id is not open).
    pub fn pin(&mut self, id: TabId) -> bool {
        if self.index_of(id).is_none() {
            return false;
        }
        self.pinned_ids.insert(id);
        true
    }

    /// Toggle the pinned state of the tab with `id`. Returns the new pinned state.
    pub fn toggle_pin(&mut self, id: TabId) -> bool {
        if self.index_of(id).is_none() {
            return false;
        }
        if self.pinned_ids.contains(&id) {
            self.pinned_ids.remove(&id);
            false
        } else {
            self.pinned_ids.insert(id);
            true
        }
    }
}

impl<T> Default for WorkspaceModel<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_model_with(n: usize) -> WorkspaceModel<&'static str> {
        let mut model = WorkspaceModel::new();
        for _ in 0..n {
            model.push("", None);
        }
        model
    }

    #[test]
    fn push_allocates_increasing_ids() {
        let mut model = WorkspaceModel::new();
        let a = model.push("a", None);
        let b = model.push("b", None);
        assert_ne!(a, b);
        assert!(a.as_u64() < b.as_u64());
    }

    #[test]
    fn active_follows_first_push() {
        let mut model = WorkspaceModel::new();
        let id = model.push("x", None);
        assert_eq!(model.active_id(), Some(id));
        assert_eq!(model.active_index(), 0);
    }

    #[test]
    fn switch_to_changes_active_and_records_history() {
        let mut model = make_model_with(3);
        let ids: Vec<_> = (0..3).map(|i| model.id_at(i).unwrap()).collect();

        // Active follows the first pushed entry; switch away from it.
        assert_eq!(model.switch_to(ids[2]), NavEffect::ActiveChanged);
        assert_eq!(model.active_index(), 2);
        assert_eq!(model.back_history(), &[ids[0]]);
    }

    #[test]
    fn switch_to_missing_id_is_noop() {
        let mut model = make_model_with(2);
        let missing = model.allocate_id();
        assert_eq!(model.switch_to(missing), NavEffect::None);
    }

    #[test]
    fn go_back_and_forward_roundtrip() {
        let mut model = make_model_with(3);
        let ids: Vec<_> = (0..3).map(|i| model.id_at(i).unwrap()).collect();

        model.switch_to(ids[2]);
        assert!(model.back_history().contains(&ids[0]));

        assert_eq!(model.go_back(), NavEffect::ActiveChanged);
        assert_eq!(model.active_id(), Some(ids[0]));
        assert!(model.forward_history().contains(&ids[2]));

        assert_eq!(model.go_forward(), NavEffect::ActiveChanged);
        assert_eq!(model.active_id(), Some(ids[2]));
    }

    #[test]
    fn close_by_id_removes_entry_and_updates_active() {
        let mut model = make_model_with(3);
        let ids: Vec<_> = (0..3).map(|i| model.id_at(i).unwrap()).collect();
        model.switch_to(ids[2]);

        let effect = model.close_by_id(ids[1]).unwrap();
        assert_eq!(effect.removed_index, 1);
        assert_eq!(model.len(), 2);
        assert!(model.index_of(ids[1]).is_none());
        // Active tab (ids[2]) should remain active because it was not closed.
        assert_eq!(model.active_id(), Some(ids[2]));
    }

    #[test]
    fn close_active_chooses_neighbor() {
        let mut model = make_model_with(3);
        let ids: Vec<_> = (0..3).map(|i| model.id_at(i).unwrap()).collect();
        model.switch_to(ids[1]);

        let effect = model.close_by_id(ids[1]).unwrap();
        assert_eq!(effect.removed_index, 1);
        assert_eq!(effect.new_active_index, 1);
        assert_eq!(model.active_id(), Some(ids[2]));
    }

    #[test]
    fn close_last_active_clears_active() {
        let mut model = WorkspaceModel::new();
        let id = model.push("only", None);
        let effect = model.close_by_id(id).unwrap();
        assert_eq!(effect.removed_index, 0);
        assert!(model.active_id().is_none());
        assert_eq!(effect.new_active_index, 0);
    }

    #[test]
    fn close_by_id_cleans_history_and_pins() {
        let mut model = make_model_with(3);
        let ids: Vec<_> = (0..3).map(|i| model.id_at(i).unwrap()).collect();
        model.toggle_pin(ids[1]);
        model.switch_to(ids[0]);

        model.close_by_id(ids[2]).unwrap();
        assert!(!model.is_pinned(ids[2]));
        assert!(model.back_history().iter().all(|&id| id != ids[2]));
    }

    #[test]
    fn pinning_is_stable_across_reordering() {
        let mut model = make_model_with(3);
        let ids: Vec<_> = (0..3).map(|i| model.id_at(i).unwrap()).collect();
        model.toggle_pin(ids[1]);
        assert!(model.is_pinned(ids[1]));

        // Close the tab before the pinned one; the pinned ID stays pinned
        // even though its index shifts from 1 to 0.
        model.close_by_id(ids[0]).unwrap();
        assert!(model.is_pinned(ids[1]));
        assert_eq!(model.index_of(ids[1]), Some(0));
    }

    #[test]
    fn entry_swap_preserves_identity_active_and_pins() {
        let mut model = make_model_with(3);
        let ids: Vec<_> = (0..3).map(|i| model.id_at(i).unwrap()).collect();
        model.toggle_pin(ids[0]);
        model.switch_to(ids[2]);

        model.entries_mut().swap(0, 2);

        assert_eq!(model.id_at(0), Some(ids[2]));
        assert_eq!(model.id_at(2), Some(ids[0]));
        assert_eq!(model.index_of(ids[0]), Some(2));
        assert_eq!(model.index_of(ids[2]), Some(0));
        assert_eq!(model.active_id(), Some(ids[2]));
        assert!(model.is_pinned(ids[0]));
    }

    #[test]
    fn index_of_and_id_at_are_inverse() {
        let mut model = WorkspaceModel::new();
        let id = model.push("x", None);
        assert_eq!(model.index_of(id), Some(0));
        assert_eq!(model.id_at(0), Some(id));
        assert_eq!(model.id_at(99), None);
        let foreign_id = model.allocate_id();
        assert_eq!(model.index_of(foreign_id), None);
    }
}
