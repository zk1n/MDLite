//! Mutable canonical Markdown source and transaction-based edit history.

use crop::{Rope, RopeSlice};
use std::{cell::OnceCell, fmt, ops::Range, sync::Arc};

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
    before: Rope,
    after: Rope,
    before_selection: Selection,
    after_selection: Selection,
    before_state: u64,
    after_state: u64,
}

/// The one editable source buffer for a document.
///
/// COW snapshots share history nodes. Selection-only movement does not revise
/// source. The lazy contiguous buffer is only for parser/save APIs and never edits.
#[derive(Debug)]
pub struct Document {
    id: DocumentId,
    source: Rope,
    contiguous: OnceCell<Arc<str>>,
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
            source: Rope::from(source),
            contiguous: OnceCell::new(),
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
        self.contiguous
            .get_or_init(|| Arc::from(self.source.to_string()))
    }

    /// Shares the once-per-revision parser/save boundary buffer.
    pub fn source_shared(&self) -> Arc<str> {
        Arc::clone(
            self.contiguous
                .get_or_init(|| Arc::from(self.source.to_string())),
        )
    }

    pub fn rope(&self) -> &Rope {
        &self.source
    }
    pub fn byte_len(&self) -> usize {
        self.source.byte_len()
    }
    /// COW snapshot is read-only and its offsets belong to this id/revision.
    pub fn snapshot(&self) -> DocumentSnapshot {
        DocumentSnapshot {
            id: self.id,
            revision: self.revision,
            source: self.source.clone(),
        }
    }
    pub fn slice(&self, range: Range<usize>) -> Result<RopeSlice<'_>, DocumentError> {
        validate_range(&self.source, &range)?;
        Ok(self.source.byte_slice(range))
    }
    pub fn byte_to_utf16(&self, byte: usize) -> Result<usize, CoordinateError> {
        coordinates::rope_byte_to_utf16(&self.source, byte)
    }
    pub fn utf16_to_byte(&self, units: usize) -> Result<usize, CoordinateError> {
        coordinates::rope_utf16_to_byte(&self.source, units)
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
        coordinates::rope_byte_to_utf16(&self.source, anchor)?;
        coordinates::rope_byte_to_utf16(&self.source, head)?;
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
    /// boundaries. An invalid endpoint discards the COW candidate before source,
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
        validate_range(&self.source, &range)?;
        let after_selection = requested_after_selection
            .unwrap_or_else(|| Selection::caret(range.start + inserted.len()));
        if self.source.byte_slice(range.clone()) == inserted {
            validate_selection(&self.source, after_selection)?;
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
        // Validate a COW candidate before publishing any source/history state.
        let mut after = self.source.clone();
        after.replace(range, inserted);
        validate_selection(&after, after_selection)?;
        let edit = Edit {
            before: self.source.clone(),
            after: after.clone(),
            before_selection: self.selection,
            after_selection,
            before_state: self.current_state,
            after_state: self.next_state,
        };
        self.source = after;
        self.contiguous.take();

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
        let Some(revision) = self.revision.checked_add(1) else {
            return false;
        };
        let edit = &self.history[self.history_cursor - 1];
        self.source = edit.before.clone();
        self.contiguous.take();
        self.selection = edit.before_selection;
        self.current_state = edit.before_state;
        self.history_cursor -= 1;
        self.revision = revision;
        true
    }

    /// Redo one edit and restore its resulting selection.
    pub fn redo(&mut self) -> bool {
        if self.history_cursor == self.history.len() {
            return false;
        }
        let Some(revision) = self.revision.checked_add(1) else {
            return false;
        };
        let edit = &self.history[self.history_cursor];
        self.source = edit.after.clone();
        self.contiguous.take();
        self.selection = edit.after_selection;
        self.current_state = edit.after_state;
        self.history_cursor += 1;
        self.revision = revision;
        true
    }

    /// Mark the current source state saved after a successful durable write.
    pub fn mark_saved(&mut self) {
        self.saved_state = Some(self.current_state);
    }
}

#[derive(Clone, Debug)]
pub struct DocumentSnapshot {
    pub id: DocumentId,
    pub revision: u64,
    source: Rope,
}
impl DocumentSnapshot {
    pub fn rope(&self) -> &Rope {
        &self.source
    }
}

fn validate_range(source: &Rope, range: &Range<usize>) -> Result<(), DocumentError> {
    if range.start > range.end {
        return Err(DocumentError::InvalidRange {
            start: range.start,
            end: range.end,
        });
    }
    coordinates::rope_byte_to_utf16(source, range.start)?;
    coordinates::rope_byte_to_utf16(source, range.end)?;
    Ok(())
}

fn validate_selection(source: &Rope, selection: Selection) -> Result<(), DocumentError> {
    for offset in [selection.anchor, selection.head] {
        coordinates::rope_byte_to_utf16(source, offset)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_and_parser_buffer_are_read_only_and_revision_owned() {
        let mut doc = Document::new(9, "a".repeat(8192));
        let snapshot = doc.snapshot();
        let input = doc.source_shared();
        doc.set_selection(10, 10).unwrap();
        assert!(Arc::ptr_eq(&input, &doc.source_shared()));
        doc.replace_range(10..11, "a").unwrap();
        assert!(Arc::ptr_eq(&input, &doc.source_shared()));
        assert!(
            doc.replace_range_with_selection(10..11, "😀", Selection::caret(11))
                .is_err()
        );
        assert!(Arc::ptr_eq(&input, &doc.source_shared()));
        doc.replace_range(10..11, "猫").unwrap();
        assert!(!Arc::ptr_eq(&input, &doc.source_shared()));
        assert_eq!(snapshot.rope(), &*input);
        assert_eq!((snapshot.id, snapshot.revision), (9, 0));
        assert!(doc.undo());
        assert_eq!(doc.rope(), snapshot.rope());
        let cached = doc.source_shared();
        let revision = doc.revision();
        assert!(doc.replace_range(8193..8193, "x").is_err());
        doc.replace_range(10..11, "a").unwrap();
        assert_eq!(doc.revision(), revision);
        assert_eq!(doc.redo_count(), 1);
        assert!(Arc::ptr_eq(&cached, &doc.source_shared()));
        assert!(doc.redo());
        assert_eq!(doc.source().get(10..13), Some("猫"));
    }

    #[test]
    fn randomized_edits_and_history_match_independent_string_states() {
        let mut doc = Document::new(10, "a😀\r\né日本語".repeat(300));
        let mut states = vec![doc.source().to_owned()];
        let mut cursor = 0;
        let mut random = 0x726f7065_u64;
        for step in 0..600 {
            random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
            match random % 7 {
                0 => {
                    let changed = doc.undo();
                    assert_eq!(changed, cursor > 0);
                    if changed {
                        cursor -= 1;
                    }
                }
                1 => {
                    let changed = doc.redo();
                    assert_eq!(changed, cursor + 1 < states.len());
                    if changed {
                        cursor += 1;
                    }
                }
                _ => {
                    let reference = &states[cursor];
                    let boundaries: Vec<_> = reference
                        .char_indices()
                        .map(|(i, _)| i)
                        .chain([reference.len()])
                        .collect();
                    let a = boundaries[(random as usize / 7) % boundaries.len()];
                    let b = boundaries[(random as usize / 97) % boundaries.len()];
                    let range = a.min(b)..a.max(b);
                    let inserted = ["猫", "😀", "\r\n", "é", "👩‍💻", ""][step % 6];
                    let mut expected = reference.clone();
                    expected.replace_range(range.clone(), inserted);
                    doc.replace_range(range, inserted).unwrap();
                    if expected != *reference {
                        states.truncate(cursor + 1);
                        states.push(expected);
                        cursor += 1;
                    }
                }
            }
            assert_eq!(doc.source(), states[cursor]);
            assert_eq!(doc.undo_count(), cursor);
            assert_eq!(doc.redo_count(), states.len() - cursor - 1);
        }
    }

    #[test]
    fn invalid_ranges_and_exhausted_revisions_are_atomic() {
        let mut doc = Document::new(1, "😀".into());
        for range in [Range { start: 4, end: 3 }, 0..5, 1..4] {
            assert!(doc.replace_range(range, "x").is_err());
            assert_eq!(doc.source(), "😀");
            assert_eq!(doc.undo_count(), 0);
        }
        doc.replace_range(0..0, "a").unwrap();
        doc.revision = u64::MAX;
        assert!(!doc.undo());
        assert!(matches!(
            doc.replace_range(0..0, "b"),
            Err(DocumentError::RevisionExhausted)
        ));
        assert_eq!(doc.source(), "a😀");
    }

    #[test]
    fn rope_save_boundary_keeps_bytes_and_failed_cp932_save_keeps_history() {
        use crate::storage::{FileFormat, FileSnapshot};
        let path = std::env::temp_dir().join(format!(
            "mdlite-rope-save-{}-{}.md",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let original = "日本語\r\nline\nlast\r";
        let mut doc = Document::new(20, original.into());
        let mut file = FileSnapshot::create_new(&path, doc.source(), FileFormat::Cp932).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        doc.set_selection(0, 0).unwrap();
        file.save(doc.source()).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        doc.replace_selection("😀").unwrap();
        let source = doc.source_shared();
        let revision = doc.revision();
        assert!(file.save(doc.source()).is_err());
        assert_eq!(doc.source(), &*source);
        assert_eq!(doc.revision(), revision);
        assert_eq!(doc.undo_count(), 1);
        assert!(doc.is_dirty());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert!(doc.undo());
        file.save(doc.source()).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert!(!doc.is_dirty());
        std::fs::remove_file(path).unwrap();
    }

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
