//! Confirmed paragraph splits retain their source separators until the boundary changes.

use std::ops::Range;
use ui::plugin::{EditPlan, EditRequest, EditSelection, EditTransaction};

#[derive(Default)]
pub(super) struct SplitHistory {
    confirmed: Vec<SplitOrigin>,
    undone: Vec<SplitOrigin>,
    pending: Option<PendingSplit>,
}

struct SplitOrigin {
    range: Range<usize>,
    inserted: String,
    original: String,
    cursor_before: usize,
    cursor_after: usize,
}

struct PendingSplit {
    replacement: Range<usize>,
    replacement_text: String,
    change: SplitChange,
}

enum SplitChange {
    Remember(SplitOrigin),
    Restore,
}

impl SplitHistory {
    pub fn synchronize(&mut self, previous: &str, current: &str) {
        if previous == current {
            return;
        }
        if self.synchronize_pending(previous, current)
            || self.synchronize_history(previous, current)
        {
            return;
        }
        let prefix = previous
            .chars()
            .zip(current.chars())
            .take_while(|(left, right)| left == right)
            .map(|(character, _)| character.len_utf8())
            .sum::<usize>();
        let suffix = previous[prefix..]
            .chars()
            .rev()
            .zip(current[prefix..].chars().rev())
            .take_while(|(left, right)| left == right)
            .map(|(character, _)| character.len_utf8())
            .sum::<usize>();
        self.transform(&(prefix..previous.len() - suffix), current.len() - prefix - suffix);
        self.undone.clear();
    }

    fn synchronize_pending(&mut self, previous: &str, current: &str) -> bool {
        let Some(pending) = self.pending.take() else {
            return false;
        };
        self.undone.clear();
        if !replacement_matches(previous, current, &pending.replacement, &pending.replacement_text)
        {
            self.confirmed.clear();
            return true;
        }
        let restored = if matches!(pending.change, SplitChange::Restore) {
            self.confirmed.pop()
        } else {
            None
        };
        self.transform(&pending.replacement, pending.replacement_text.len());
        if let Some(origin) = restored {
            self.undone.push(origin);
        }
        if let SplitChange::Remember(origin) = pending.change {
            self.confirmed.push(origin);
        }
        true
    }

    fn synchronize_history(&mut self, previous: &str, current: &str) -> bool {
        if self.confirmed.last().is_some_and(|origin| {
            replacement_matches(previous, current, &origin.range, &origin.original)
        }) {
            let origin = self.confirmed.pop().expect("matching confirmed origin was checked");
            self.transform(&origin.range, origin.original.len());
            self.undone.push(origin);
            return true;
        }
        if self.undone.last().is_some_and(|origin| {
            replacement_matches(
                previous,
                current,
                &(origin.range.start..origin.range.start + origin.original.len()),
                &origin.inserted,
            )
        }) {
            let origin = self.undone.pop().expect("matching undone origin was checked");
            self.transform(
                &(origin.range.start..origin.range.start + origin.original.len()),
                origin.inserted.len(),
            );
            self.confirmed.push(origin);
            return true;
        }
        false
    }

    fn transform(&mut self, replacement: &Range<usize>, inserted_length: usize) {
        self.confirmed.retain_mut(|origin| {
            if replacement.start >= origin.range.end {
                return true;
            }
            if replacement.end > origin.range.start {
                return false;
            }
            let shift = |offset: usize| offset - replacement.len() + inserted_length;
            origin.range = shift(origin.range.start)..shift(origin.range.end);
            origin.cursor_before = shift(origin.cursor_before);
            origin.cursor_after = shift(origin.cursor_after);
            true
        });
    }

    pub fn begin_request(&mut self) {
        self.pending = None;
    }

    pub fn remember(&mut self, source: &str, request: &EditRequest, plan: &EditPlan) {
        let EditPlan::Apply(transaction) = plan else {
            return;
        };
        let [replacement] = transaction.replacements.as_slice() else {
            return;
        };
        let EditSelection::Caret(cursor_after) = transaction.selection_after else {
            return;
        };
        if !replacement.text.contains('\n') {
            return;
        }
        let origin = SplitOrigin {
            range: replacement.range.start..replacement.range.start + replacement.text.len(),
            inserted: replacement.text.clone(),
            original: source[replacement.range.clone()].to_owned(),
            cursor_before: request.cursor_byte,
            cursor_after,
        };
        self.pending = Some(Self::pending(
            replacement.range.clone(),
            &replacement.text,
            SplitChange::Remember(origin),
        ));
    }

    pub fn restore(&mut self, source: &str, request: &EditRequest) -> Option<EditPlan> {
        let origin = self.confirmed.last()?;
        if request.cursor_byte != origin.cursor_after
            || source.get(origin.range.clone())? != origin.inserted
        {
            return None;
        }
        let transaction = EditTransaction::replace(
            request.source_generation,
            origin.range.clone(),
            origin.original.clone(),
            origin.cursor_before,
        );
        self.pending =
            Some(Self::pending(origin.range.clone(), &origin.original, SplitChange::Restore));
        Some(EditPlan::Apply(transaction))
    }

    fn pending(replacement: Range<usize>, inserted: &str, change: SplitChange) -> PendingSplit {
        PendingSplit { replacement, replacement_text: inserted.to_owned(), change }
    }
}

fn replacement_matches(previous: &str, current: &str, range: &Range<usize>, text: &str) -> bool {
    let current_end = range.start + text.len();
    previous.get(..range.start).is_some_and(|prefix| Some(prefix) == current.get(..range.start))
        && current.get(range.start..current_end) == Some(text)
        && previous
            .get(range.end..)
            .is_some_and(|suffix| Some(suffix) == current.get(current_end..))
}
