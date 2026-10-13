// Native authoring commands use canonical UTF-8 source and selections.
use super::{LiveEdit, Selection};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

/// Unicode UAX #29 segments, including punctuation/emoji; whitespace is skipped
/// toward the next segment. Never split a grapheme or the middle of CRLF.
pub(super) fn word_target(source: &str, p: usize, back: bool) -> usize {
    let mut segments = source
        .split_word_bound_indices()
        .filter(|(_, text)| !text.chars().all(char::is_whitespace))
        .map(|(start, text)| (start, start + text.len()));
    if back {
        segments
            .rev()
            .find(|&(start, _)| start < p)
            .map_or(0, |(start, _)| start)
    } else {
        match segments.find(|&(_, end)| end > p) {
            Some((start, _)) if start > p => start,
            Some(_) => segments.next().map_or(source.len(), |(start, _)| start),
            None => source.len(),
        }
    }
}
/// Ctrl+Up/Down visit physical logical-line starts, including blank lines.
pub(super) fn logical_line_target(source: &str, p: usize, back: bool) -> usize {
    if back {
        let start = line_start(source, p);
        if p > start {
            start
        } else {
            let mut previous = start.saturating_sub(1);
            if source.as_bytes().get(previous) == Some(&b'\n')
                && previous > 0
                && source.as_bytes()[previous - 1] == b'\r'
            {
                previous -= 1;
            }
            line_start(source, previous)
        }
    } else {
        source[p..]
            .find(['\r', '\n'])
            .map_or(source.len(), |offset| {
                let start = p + offset;
                start
                    + if source[start..].starts_with("\r\n") {
                        2
                    } else {
                        1
                    }
            })
    }
}
pub(super) fn physical_line_starts(source: &str) -> Vec<usize> {
    let mut starts = vec![0];
    let bytes = source.as_bytes();
    for (index, &byte) in bytes.iter().enumerate() {
        if byte == b'\n' || (byte == b'\r' && bytes.get(index + 1) != Some(&b'\n')) {
            starts.push(index + 1);
        }
    }
    starts
}
pub(super) fn line_start(source: &str, p: usize) -> usize {
    source[..p].rfind(['\r', '\n']).map_or(0, |index| index + 1)
}
pub(super) fn enter_edit(source: &str, selection: Selection, literal: bool) -> LiveEdit {
    let range = selection.range();
    let p = range.start;
    let start = line_start(source, p);
    let line_end = source[start..]
        .find(['\r', '\n'])
        .map_or(source.len(), |end| start + end);
    let line = &source[start..line_end];
    let eol = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let indent = &line[..line.len() - line.trim_start_matches([' ', '\t']).len()];
    let mut prefix = indent.to_owned();
    if !literal {
        let mut rest = &line[indent.len()..];
        while let Some(quoted) = rest.strip_prefix('>') {
            prefix.push('>');
            rest = quoted;
            if let Some(after_space) = rest.strip_prefix(' ') {
                prefix.push(' ');
                rest = after_space;
            }
        }
        let quote_len = prefix.len();
        if rest.starts_with(['-', '+', '*']) && rest.as_bytes().get(1) == Some(&b' ') {
            prefix.push_str(&rest[..2]);
            rest = &rest[2..];
        } else {
            let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
            if digits > 0
                && matches!(rest.as_bytes().get(digits), Some(b'.' | b')'))
                && rest.as_bytes().get(digits + 1) == Some(&b' ')
            {
                if let Ok(number) = rest[..digits].parse::<u64>() {
                    prefix.push_str(&format!(
                        "{}{} ",
                        number.saturating_add(1),
                        &rest[digits..digits + 1]
                    ));
                }
                rest = &rest[digits + 2..];
            }
        }
        if prefix.len() > quote_len && matches!(rest.get(..4), Some("[ ] " | "[x] " | "[X] ")) {
            prefix.push_str("[ ] ");
            rest = &rest[4..];
        }
        if selection.is_caret()
            && rest.trim().is_empty()
            && p == line_end
            && prefix.len() > indent.len()
        {
            return LiveEdit {
                range: start..p,
                replacement: indent.to_owned(),
                caret: start + indent.len(),
            };
        }
    }
    let replacement = format!("{eol}{prefix}");
    LiveEdit {
        caret: p + replacement.len(),
        range,
        replacement,
    }
}
pub(super) fn indentation_edit(
    source: &str,
    selection: Selection,
    back: bool,
) -> (Range<usize>, String, Selection) {
    let selected = selection.range();
    let start = line_start(source, selected.start);
    let last = if selected.end > selected.start && line_start(source, selected.end) == selected.end
    {
        selected.end - 1
    } else {
        selected.end
    };
    let end = source[last..]
        .find(['\r', '\n'])
        .map_or(source.len(), |offset| last + offset);
    let mut replacement = String::new();
    let mut changes = Vec::new();
    let mut cursor = start;
    for line in source[start..end].split_inclusive('\n') {
        let removed = if back {
            if line.starts_with('\t') {
                1
            } else {
                line.bytes()
                    .take_while(|&byte| byte == b' ')
                    .take(4)
                    .count()
            }
        } else {
            0
        };
        changes.push((cursor, removed, if back { 0 } else { 4 }));
        if !back {
            replacement.push_str("    ");
        }
        replacement.push_str(&line[removed..]);
        cursor += line.len();
    }
    if start == end && !back {
        replacement.push_str("    ");
        changes.push((start, 0, 4));
    }
    let relocate = |p: usize| {
        let mut offset = p as isize;
        for &(position, removed, inserted) in &changes {
            if p >= position {
                offset += inserted as isize - removed.min(p - position) as isize;
            }
        }
        offset as usize
    };
    (
        start..end,
        replacement,
        Selection {
            anchor: relocate(selection.anchor),
            head: relocate(selection.head),
        },
    )
}
