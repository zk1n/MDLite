//! Mutable canonical Markdown source and transaction-based edit history.

use std::{fmt, ops::Range};

use crate::coordinates::{self, CoordinateError};

pub type DocumentId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

impl Selection {
    pub const fn caret(byte_offset: usize) -> Self {
        Self {
            anchor: byte_offset,
            head: byte_offset,
        }
    }

    pub fn range(self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    pub const fn is_caret(self) -> bool {
        self.anchor == self.head
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentError {
    Coordinates(CoordinateError),
    InvalidRange { start: usize, end: usize },
    RevisionExhausted,
}

impl fmt::Display for DocumentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Coordinates(error) => error.fmt(f),
            Self::InvalidRange { start, end } => {
                write!(f, "edit range {start}..{end} is reversed")
            }
            Self::RevisionExhausted => f.write_str("document revision counter is exhausted"),
        }
    }
}

impl std::error::Error for DocumentError {}

impl From<CoordinateError> for DocumentError {
    fn from(value: CoordinateError) -> Self {
        Self::Coordinates(value)
    }
}

#[derive(Debug)]
struct Edit {
    start: usize,
    removed: String,
    inserted: String,
    before_selection: Selection,
    after_selection: Selection,
    before_state: u64,
    after_state: u64,
}

/// The one editable source buffer for a document.
///
/// Edits store only their removed/inserted slices; the document source is never
/// cloned to build history. Selection-only movement does not change revision or
/// dirty state.
#[derive(Debug)]
pub struct Document {
    id: DocumentId,
    source: String,
    selection: Selection,
    revision: u64,
    current_state: u64,
    saved_state: Option<u64>,
    next_state: u64,
    history: Vec<Edit>,
    history_cursor: usize,
}

impl Document {
    /// Create a clean document, typically from a decoded file.
    pub fn new(id: DocumentId, source: String) -> Self {
        Self::with_saved_state(id, source, Some(0))
    }

    /// Create a new untitled document that needs its first save.
    pub fn new_unsaved(id: DocumentId, source: String) -> Self {
        Self::with_saved_state(id, source, None)
    }

    fn with_saved_state(id: DocumentId, source: String, saved_state: Option<u64>) -> Self {
        Self {
            id,
            source,
            selection: Selection::caret(0),
            revision: 0,
            current_state: 0,
            saved_state,
            next_state: 1,
            history: Vec::new(),
            history_cursor: 0,
        }
    }

    pub const fn id(&self) -> DocumentId {
        self.id
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub const fn selection(&self) -> Selection {
        self.selection
    }

    pub const fn revision(&self) -> u64 {
        self.revision
    }

    pub fn is_dirty(&self) -> bool {
        Some(self.current_state) != self.saved_state
    }

    pub fn undo_count(&self) -> usize {
        self.history_cursor
    }

    pub fn redo_count(&self) -> usize {
        self.history.len() - self.history_cursor
    }

    /// Set a selection using UTF-8 byte offsets. Both endpoints must be scalar boundaries.
    pub fn set_selection(&mut self, anchor: usize, head: usize) -> Result<(), DocumentError> {
        coordinates::byte_to_utf16(&self.source, anchor)?;
        coordinates::byte_to_utf16(&self.source, head)?;
        self.selection = Selection { anchor, head };
        Ok(())
    }

    /// Replace the active selection as one undoable transaction.
    pub fn replace_selection(&mut self, inserted: &str) -> Result<(), DocumentError> {
        self.replace_range(self.selection.range(), inserted)
    }

    /// Replace one UTF-8 byte range as one undoable transaction.
    ///
    /// This supports IME commits, paste, and structural edits as a single action.
    pub fn replace_range(
        &mut self,
        range: Range<usize>,
        inserted: &str,
    ) -> Result<(), DocumentError> {
        self.replace_range_inner(range, inserted, None)
    }

    /// Replace one source range and restore an explicitly chosen selection after the edit.
    ///
    /// The requested selection is checked against the resulting source's UTF-8 scalar
    /// boundaries. If an endpoint is invalid, the source splice is rolled back before
    /// history or document state changes.
    pub fn replace_range_with_selection(
        &mut self,
        range: Range<usize>,
        inserted: &str,
        after_selection: Selection,
    ) -> Result<(), DocumentError> {
        self.replace_range_inner(range, inserted, Some(after_selection))
    }

    fn replace_range_inner(
        &mut self,
        range: Range<usize>,
        inserted: &str,
        requested_after_selection: Option<Selection>,
    ) -> Result<(), DocumentError> {
        if range.start > range.end {
            return Err(DocumentError::InvalidRange {
                start: range.start,
                end: range.end,
            });
        }
        coordinates::byte_to_utf16(&self.source, range.start)?;
        coordinates::byte_to_utf16(&self.source, range.end)?;

        let removed = self.source[range.clone()].to_owned();
        let after_selection = requested_after_selection
            .unwrap_or_else(|| Selection::caret(range.start + inserted.len()));
        if removed == inserted {
            if requested_after_selection.is_some() {
                validate_selection(&self.source, after_selection)?;
            }
            self.selection = after_selection;
            return Ok(());
        }

        let next_revision = self
            .revision
            .checked_add(1)
            .ok_or(DocumentError::RevisionExhausted)?;
        let next_state = self
            .next_state
            .checked_add(1)
            .ok_or(DocumentError::RevisionExhausted)?;
        let edit = Edit {
            start: range.start,
            removed,
            inserted: inserted.to_owned(),
            before_selection: self.selection,
            after_selection,
            before_state: self.current_state,
            after_state: self.next_state,
        };

        let inserted_end = range.start + inserted.len();
        self.source.replace_range(range.clone(), inserted);
        if requested_after_selection.is_some()
            && let Err(error) = validate_selection(&self.source, after_selection)
        {
            self.source
                .replace_range(range.start..inserted_end, &edit.removed);
            return Err(error);
        }

        self.history.truncate(self.history_cursor);
        self.history.push(edit);
        self.history_cursor += 1;
        self.selection = after_selection;
        self.current_state = self.next_state;
        self.next_state = next_state;
        self.revision = next_revision;
        Ok(())
    }

    /// Undo one edit and restore its original selection.
    pub fn undo(&mut self) -> bool {
        if self.history_cursor == 0 {
            return false;
        }
        let edit = &self.history[self.history_cursor - 1];
        let end = edit.start + edit.inserted.len();
        self.source.replace_range(edit.start..end, &edit.removed);
        self.selection = edit.before_selection;
        self.current_state = edit.before_state;
        self.history_cursor -= 1;
        self.revision = self.revision.saturating_add(1);
        true
    }

    /// Redo one edit and restore its resulting selection.
    pub fn redo(&mut self) -> bool {
        if self.history_cursor == self.history.len() {
            return false;
        }
        let edit = &self.history[self.history_cursor];
        let end = edit.start + edit.removed.len();
        self.source.replace_range(edit.start..end, &edit.inserted);
        self.selection = edit.after_selection;
        self.current_state = edit.after_state;
        self.history_cursor += 1;
        self.revision = self.revision.saturating_add(1);
        true
    }

    /// Mark the current source state saved after a successful durable write.
    pub fn mark_saved(&mut self) {
        self.saved_state = Some(self.current_state);
    }
}

fn validate_selection(source: &str, selection: Selection) -> Result<(), DocumentError> {
    for offset in [selection.anchor, selection.head] {
        coordinates::byte_to_utf16(source, offset)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_undo_redo_restores_source_selection_and_saved_state() {
        let mut doc = Document::new(41, "A😀B".to_owned());
        doc.set_selection(1, 5).unwrap();
        let before_revision = doc.revision();
        doc.replace_selection("猫\r\n日本語").unwrap();
        let after = "A猫\r\n日本語B";
        assert_eq!(doc.source(), after);
        assert_eq!(doc.selection(), Selection::caret(1 + "猫\r\n日本語".len()));
        assert_eq!(doc.undo_count(), 1);
        assert!(doc.is_dirty());
        assert!(doc.revision() > before_revision);

        doc.mark_saved();
        assert!(!doc.is_dirty());
        assert!(doc.undo());
        assert_eq!(doc.source(), "A😀B");
        assert_eq!(doc.selection(), Selection { anchor: 1, head: 5 });
        assert!(doc.is_dirty());
        assert!(doc.redo());
        assert_eq!(doc.source(), after);
        assert!(!doc.is_dirty());
    }

    #[test]
    fn selection_movement_and_noop_edits_do_not_dirty_or_revise() {
        let mut doc = Document::new(7, "hello".to_owned());
        doc.set_selection(1, 4).unwrap();
        assert_eq!(doc.revision(), 0);
        assert!(!doc.is_dirty());
        doc.replace_selection("ell").unwrap();
        assert_eq!(doc.source(), "hello");
        assert_eq!(doc.selection(), Selection::caret(4));
        assert_eq!(doc.revision(), 0);
        assert!(!doc.is_dirty());
        assert_eq!(doc.undo_count(), 0);
    }

    #[test]
    fn each_range_replacement_is_one_transaction_and_branches_drop_redo() {
        let mut doc = Document::new(1, String::new());
        doc.replace_range(0..0, "日本語").unwrap();
        assert_eq!(doc.undo_count(), 1);
        assert!(doc.undo());
        doc.replace_range(0..0, "別").unwrap();
        assert_eq!(doc.redo_count(), 0);
        assert_eq!(doc.source(), "別");
    }

    #[test]
    fn offsets_are_checked_before_mutation() {
        let mut doc = Document::new(1, "😀".to_owned());
        assert!(doc.replace_range(1..4, "x").is_err());
        assert_eq!(doc.source(), "😀");
        assert_eq!(doc.revision(), 0);
    }

    #[test]
    fn range_edit_can_restore_custom_caret_while_preserving_hidden_markdown() {
        let mut doc = Document::new(12, "**old** gap".to_owned());
        let before = Selection { anchor: 5, head: 2 };
        doc.set_selection(before.anchor, before.head).unwrap();
        let after = Selection::caret(6);

        doc.replace_range_with_selection(2..5, "replacement", after)
            .unwrap();
        assert_eq!(doc.source(), "**replacement** gap");
        assert_eq!(doc.selection(), after);
        assert_eq!(doc.undo_count(), 1);

        assert!(doc.undo());
        assert_eq!(doc.source(), "**old** gap");
        assert_eq!(doc.selection(), before);
        assert!(doc.redo());
        assert_eq!(doc.source(), "**replacement** gap");
        assert_eq!(doc.selection(), after);
    }

    #[test]
    fn invalid_custom_selection_rolls_back_without_changing_document_state() {
        let mut doc = Document::new(13, "**a**".to_owned());
        let before = Selection { anchor: 3, head: 2 };
        doc.set_selection(before.anchor, before.head).unwrap();
        let invalid_after = Selection::caret(3);

        assert!(matches!(
            doc.replace_range_with_selection(2..3, "😀", invalid_after),
            Err(DocumentError::Coordinates(
                CoordinateError::NotCharBoundary { offset: 3 }
            ))
        ));
        assert_eq!(doc.source(), "**a**");
        assert_eq!(doc.selection(), before);
        assert_eq!(doc.revision(), 0);
        assert!(!doc.is_dirty());
        assert_eq!(doc.undo_count(), 0);
        assert_eq!(doc.redo_count(), 0);
    }
}
