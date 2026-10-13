//! Checked conversions between canonical UTF-8 source offsets and editor coordinates.

use std::fmt;

use unicode_segmentation::UnicodeSegmentation;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoordinateError {
    ByteOffsetOutOfBounds { offset: usize, len: usize },
    NotCharBoundary { offset: usize },
    Utf16OffsetOutOfBounds { offset: usize, len: usize },
    InsideSurrogatePair { offset: usize },
    LineOutOfBounds { line: usize, count: usize },
    Utf16ColumnOutOfBounds { column: usize, len: usize },
    InsideLineEnding { offset: usize },
}

impl fmt::Display for CoordinateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ByteOffsetOutOfBounds { offset, len } => {
                write!(f, "UTF-8 byte offset {offset} exceeds source length {len}")
            }
            Self::NotCharBoundary { offset } => {
                write!(f, "UTF-8 byte offset {offset} splits a Unicode scalar")
            }
            Self::Utf16OffsetOutOfBounds { offset, len } => {
                write!(f, "UTF-16 offset {offset} exceeds source length {len}")
            }
            Self::InsideSurrogatePair { offset } => {
                write!(f, "UTF-16 offset {offset} splits a surrogate pair")
            }
            Self::LineOutOfBounds { line, count } => {
                write!(f, "line {line} is outside the {count}-line source")
            }
            Self::Utf16ColumnOutOfBounds { column, len } => {
                write!(f, "UTF-16 column {column} exceeds line length {len}")
            }
            Self::InsideLineEnding { offset } => {
                write!(f, "byte offset {offset} is between CR and LF")
            }
        }
    }
}

impl std::error::Error for CoordinateError {}

/// Zero-based source line and UTF-16 column, excluding line-ending bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineColumn {
    pub line: usize,
    pub utf16_column: usize,
}

/// Checked Rope coordinates use crop's metrics without a contiguous copy.
pub fn rope_byte_to_utf16(text: &crop::Rope, byte: usize) -> Result<usize, CoordinateError> {
    check_rope_byte(text, byte)?;
    Ok(text.utf16_code_unit_of_byte(byte))
}
pub fn rope_utf16_to_byte(text: &crop::Rope, units: usize) -> Result<usize, CoordinateError> {
    if units > text.utf16_len() {
        return Err(CoordinateError::Utf16OffsetOutOfBounds {
            offset: units,
            len: text.utf16_len(),
        });
    }
    let byte = text.byte_of_utf16_code_unit(units);
    if text.utf16_code_unit_of_byte(byte) != units {
        return Err(CoordinateError::InsideSurrogatePair { offset: units });
    }
    Ok(byte)
}
fn check_rope_byte(text: &crop::Rope, byte: usize) -> Result<(), CoordinateError> {
    if byte > text.byte_len() {
        return Err(CoordinateError::ByteOffsetOutOfBounds {
            offset: byte,
            len: text.byte_len(),
        });
    }
    if !text.is_char_boundary(byte) {
        return Err(CoordinateError::NotCharBoundary { offset: byte });
    }
    Ok(())
}
/// crop joins graphemes across chunks; chunks are never segmented separately.
pub fn rope_is_grapheme_boundary(text: &crop::Rope, byte: usize) -> Result<bool, CoordinateError> {
    check_rope_byte(text, byte)?;
    Ok(text.is_grapheme_boundary(byte))
}
pub fn rope_previous_grapheme_boundary(
    text: &crop::Rope,
    byte: usize,
) -> Result<usize, CoordinateError> {
    check_rope_byte(text, byte)?;
    // crop 0.4.3's iterator loses regional-indicator context after yielding a
    // cluster. Its boundary API consults the original chunks and keeps parity.
    Ok((0..byte)
        .rev()
        .find(|&candidate| text.is_char_boundary(candidate) && text.is_grapheme_boundary(candidate))
        .unwrap_or(0))
}
pub fn rope_next_grapheme_boundary(
    text: &crop::Rope,
    byte: usize,
) -> Result<usize, CoordinateError> {
    check_rope_byte(text, byte)?;
    Ok((byte..text.byte_len())
        .map(|candidate| candidate + 1)
        .find(|&candidate| text.is_char_boundary(candidate) && text.is_grapheme_boundary(candidate))
        .unwrap_or(text.byte_len()))
}

fn check_byte_offset(text: &str, byte_offset: usize) -> Result<(), CoordinateError> {
    if byte_offset > text.len() {
        return Err(CoordinateError::ByteOffsetOutOfBounds {
            offset: byte_offset,
            len: text.len(),
        });
    }
    if !text.is_char_boundary(byte_offset) {
        return Err(CoordinateError::NotCharBoundary {
            offset: byte_offset,
        });
    }
    Ok(())
}

/// Convert a UTF-8 byte offset to the corresponding UTF-16 code-unit offset.
pub fn byte_to_utf16(text: &str, byte_offset: usize) -> Result<usize, CoordinateError> {
    check_byte_offset(text, byte_offset)?;
    Ok(text[..byte_offset].encode_utf16().count())
}

/// Convert an exact UTF-16 code-unit offset to a UTF-8 byte offset.
///
/// Offsets inside a supplementary character's surrogate pair are rejected.
pub fn utf16_to_byte(text: &str, utf16_offset: usize) -> Result<usize, CoordinateError> {
    let total = text.encode_utf16().count();
    if utf16_offset > total {
        return Err(CoordinateError::Utf16OffsetOutOfBounds {
            offset: utf16_offset,
            len: total,
        });
    }

    let mut units = 0;
    for (byte, ch) in text.char_indices() {
        if units == utf16_offset {
            return Ok(byte);
        }
        units += ch.len_utf16();
        if units > utf16_offset {
            return Err(CoordinateError::InsideSurrogatePair {
                offset: utf16_offset,
            });
        }
    }
    Ok(text.len())
}

/// Return whether a valid UTF-8 byte offset is also an extended grapheme boundary.
pub fn is_grapheme_boundary(text: &str, byte_offset: usize) -> Result<bool, CoordinateError> {
    check_byte_offset(text, byte_offset)?;
    Ok(byte_offset == text.len()
        || text
            .grapheme_indices(true)
            .any(|(start, _)| start == byte_offset))
}

/// Return the previous grapheme boundary, or zero at the start of the source.
///
/// If `byte_offset` is inside a grapheme, this returns that grapheme's start.
pub fn previous_grapheme_boundary(
    text: &str,
    byte_offset: usize,
) -> Result<usize, CoordinateError> {
    check_byte_offset(text, byte_offset)?;
    if byte_offset == 0 {
        return Ok(0);
    }

    for (start, grapheme) in text.grapheme_indices(true) {
        if byte_offset <= start + grapheme.len() {
            return Ok(start);
        }
    }
    Ok(text.len())
}

/// Return the next grapheme boundary, or the source length at the end.
///
/// If `byte_offset` is inside a grapheme, this returns that grapheme's end.
pub fn next_grapheme_boundary(text: &str, byte_offset: usize) -> Result<usize, CoordinateError> {
    check_byte_offset(text, byte_offset)?;
    for (start, grapheme) in text.grapheme_indices(true) {
        let end = start + grapheme.len();
        if start < byte_offset && byte_offset < end {
            return Ok(end);
        }
        if start > byte_offset {
            return Ok(start);
        }
    }
    Ok(text.len())
}

fn line_bounds(text: &str, wanted: usize) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut line = 0;
    let mut start = 0;
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'\r' => {
                if line == wanted {
                    return Some((start, i));
                }
                line += 1;
                if bytes.get(i + 1) == Some(&b'\n') {
                    i += 2;
                } else {
                    i += 1;
                }
                start = i;
            }
            b'\n' => {
                if line == wanted {
                    return Some((start, i));
                }
                line += 1;
                i += 1;
                start = i;
            }
            _ => i += 1,
        }
    }

    (line == wanted).then_some((start, text.len()))
}

fn line_count(text: &str) -> usize {
    let bytes = text.as_bytes();
    let mut lines = 1;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\r' => {
                lines += 1;
                i += if bytes.get(i + 1) == Some(&b'\n') {
                    2
                } else {
                    1
                };
            }
            b'\n' => {
                lines += 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
    lines
}

/// Convert a source byte offset to a zero-based line and UTF-16 column.
///
/// CRLF is one line ending. An offset between its CR and LF bytes is rejected;
/// lone CR and lone LF each start a new line.
pub fn byte_to_line_column(text: &str, byte_offset: usize) -> Result<LineColumn, CoordinateError> {
    check_byte_offset(text, byte_offset)?;

    let bytes = text.as_bytes();
    let mut line = 0;
    let mut line_start = 0;
    let mut i = 0;
    while i < byte_offset {
        match bytes[i] {
            b'\r' if bytes.get(i + 1) == Some(&b'\n') => {
                if byte_offset == i + 1 {
                    return Err(CoordinateError::InsideLineEnding {
                        offset: byte_offset,
                    });
                }
                line += 1;
                i += 2;
                line_start = i;
            }
            b'\r' | b'\n' => {
                line += 1;
                i += 1;
                line_start = i;
            }
            _ => {
                let ch = text[i..].chars().next().expect("valid source boundary");
                i += ch.len_utf8();
            }
        }
    }

    Ok(LineColumn {
        line,
        utf16_column: byte_to_utf16(&text[line_start..byte_offset], byte_offset - line_start)?,
    })
}

/// Convert a zero-based source line and UTF-16 column to a UTF-8 byte offset.
pub fn line_column_to_byte(
    text: &str,
    line: usize,
    utf16_column: usize,
) -> Result<usize, CoordinateError> {
    let (start, end) = line_bounds(text, line).ok_or(CoordinateError::LineOutOfBounds {
        line,
        count: line_count(text),
    })?;
    let line_text = &text[start..end];
    let line_len = line_text.encode_utf16().count();
    if utf16_column > line_len {
        return Err(CoordinateError::Utf16ColumnOutOfBounds {
            column: utf16_column,
            len: line_len,
        });
    }
    Ok(start + utf16_to_byte(line_text, utf16_column)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rope_metrics_and_graphemes_cross_real_chunks_without_flattening() {
        let text = format!("Ae{}👨‍👩‍👧‍👦\u{fe0f}\r\n日本語😀", "\u{301}".repeat(4000));
        let rope = crop::Rope::from(text.as_str());
        assert!(rope.chunks().count() > 4);
        let mut chunk_end = 0;
        assert!(rope.chunks().any(|chunk| {
            chunk_end += chunk.len();
            chunk_end < text.len() && !is_grapheme_boundary(&text, chunk_end).unwrap()
        }));
        for byte in text.char_indices().map(|(i, _)| i).chain([text.len()]) {
            let units = rope_byte_to_utf16(&rope, byte).unwrap();
            assert_eq!(units, byte_to_utf16(&text, byte).unwrap());
            assert_eq!(rope_utf16_to_byte(&rope, units).unwrap(), byte);
        }
        for byte in [0, 1, 3, 8002, text.len()] {
            assert_eq!(
                rope_is_grapheme_boundary(&rope, byte),
                is_grapheme_boundary(&text, byte)
            );
            assert_eq!(
                rope_previous_grapheme_boundary(&rope, byte),
                previous_grapheme_boundary(&text, byte)
            );
            assert_eq!(
                rope_next_grapheme_boundary(&rope, byte),
                next_grapheme_boundary(&text, byte)
            );
        }
        let emoji = text.len() - 4;
        let units = rope_byte_to_utf16(&rope, emoji).unwrap();
        assert!(matches!(
            rope_utf16_to_byte(&rope, units + 1),
            Err(CoordinateError::InsideSurrogatePair { .. })
        ));
        assert!(rope_byte_to_utf16(&rope, emoji + 1).is_err());
        assert!(rope_utf16_to_byte(&rope, rope.utf16_len() + 1).is_err());
        for sample in ["a🇦🇧🇨🇩🇪z", "a👩‍💻\u{fe0f}z", "a\r\nz", "aक्‍षz"]
        {
            let rope = crop::Rope::from(sample);
            for byte in sample.char_indices().map(|(i, _)| i).chain([sample.len()]) {
                assert_eq!(
                    rope_is_grapheme_boundary(&rope, byte),
                    is_grapheme_boundary(sample, byte)
                );
                assert_eq!(
                    rope_previous_grapheme_boundary(&rope, byte),
                    previous_grapheme_boundary(sample, byte),
                    "sample={sample:?}, byte={byte}, crop={:?}",
                    rope.graphemes().collect::<Vec<_>>()
                );
                assert_eq!(
                    rope_next_grapheme_boundary(&rope, byte),
                    next_grapheme_boundary(sample, byte),
                    "sample={sample:?}, byte={byte}, crop={:?}",
                    rope.graphemes().collect::<Vec<_>>()
                );
            }
        }
        let regional = format!("{}{}z", "a".repeat(1020), "🇦🇧🇨🇩🇪".repeat(600));
        let rope = crop::Rope::from(regional.as_str());
        let boundaries: Vec<_> = regional
            .grapheme_indices(true)
            .map(|(byte, _)| byte)
            .chain([regional.len()])
            .collect();
        assert!(rope.chunks().count() > 8);
        for byte in regional
            .char_indices()
            .map(|(byte, _)| byte)
            .chain([regional.len()])
        {
            assert_eq!(
                rope_is_grapheme_boundary(&rope, byte).unwrap(),
                boundaries.binary_search(&byte).is_ok(),
                "regional byte={byte}"
            );
        }
        let mut end = 0;
        for chunk in rope.chunks() {
            end += chunk.len();
            assert_eq!(
                rope_previous_grapheme_boundary(&rope, end).unwrap(),
                boundaries
                    .iter()
                    .copied()
                    .take_while(|&byte| byte < end)
                    .last()
                    .unwrap_or(0)
            );
            assert_eq!(
                rope_next_grapheme_boundary(&rope, end).unwrap(),
                boundaries
                    .iter()
                    .copied()
                    .find(|&byte| byte > end)
                    .unwrap_or(regional.len())
            );
        }
    }

    #[test]
    fn utf8_byte_and_utf16_offsets_round_trip_unicode() {
        let text = "A😀日本語";
        for byte in text
            .char_indices()
            .map(|(byte, _)| byte)
            .chain([text.len()])
        {
            let utf16 = byte_to_utf16(text, byte).unwrap();
            assert_eq!(utf16_to_byte(text, utf16).unwrap(), byte);
            assert_eq!(utf16, text[..byte].encode_utf16().count());
        }
        assert!(matches!(
            utf16_to_byte(text, 2),
            Err(CoordinateError::InsideSurrogatePair { .. })
        ));
        assert!(matches!(
            byte_to_utf16(text, 2),
            Err(CoordinateError::NotCharBoundary { .. })
        ));
    }

    #[test]
    fn mixed_line_endings_round_trip_and_reject_crlf_middle() {
        let text = "a😀\r\nb\nc\rd";
        let expected = [
            (
                0,
                LineColumn {
                    line: 0,
                    utf16_column: 0,
                },
            ),
            (
                1,
                LineColumn {
                    line: 0,
                    utf16_column: 1,
                },
            ),
            (
                5,
                LineColumn {
                    line: 0,
                    utf16_column: 3,
                },
            ),
            (
                7,
                LineColumn {
                    line: 1,
                    utf16_column: 0,
                },
            ),
            (
                8,
                LineColumn {
                    line: 1,
                    utf16_column: 1,
                },
            ),
            (
                9,
                LineColumn {
                    line: 2,
                    utf16_column: 0,
                },
            ),
            (
                10,
                LineColumn {
                    line: 2,
                    utf16_column: 1,
                },
            ),
            (
                11,
                LineColumn {
                    line: 3,
                    utf16_column: 0,
                },
            ),
            (
                12,
                LineColumn {
                    line: 3,
                    utf16_column: 1,
                },
            ),
        ];
        for (byte, point) in expected {
            assert_eq!(byte_to_line_column(text, byte).unwrap(), point);
            assert_eq!(
                line_column_to_byte(text, point.line, point.utf16_column).unwrap(),
                byte
            );
        }
        assert!(matches!(
            byte_to_line_column(text, 6),
            Err(CoordinateError::InsideLineEnding { .. })
        ));
    }

    #[test]
    fn grapheme_navigation_skips_combining_and_joined_emoji_sequences() {
        let text = "e\u{301}👨‍👩‍👧‍👦x";
        let boundaries: Vec<_> = text
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .chain([text.len()])
            .collect();
        assert_eq!(boundaries.len(), 4);
        assert!(is_grapheme_boundary(text, boundaries[1]).unwrap());
        assert!(!is_grapheme_boundary(text, boundaries[1] + 4).unwrap());
        assert_eq!(
            next_grapheme_boundary(text, boundaries[0]).unwrap(),
            boundaries[1]
        );
        assert_eq!(
            next_grapheme_boundary(text, boundaries[1]).unwrap(),
            boundaries[2]
        );
        assert_eq!(
            previous_grapheme_boundary(text, boundaries[2]).unwrap(),
            boundaries[1]
        );
    }
}
