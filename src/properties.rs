//! Bounded YAML front matter projection. Source ranges, never a YAML dump, are edited.
use std::{collections::HashSet, ops::Range};

use crate::{
    document::{Document, DocumentError, DocumentId, Selection},
    projection::{BlockKind, DisplayProjection},
};

const MAX_BYTES: usize = 256 * 1024;
const MAX_ENTRIES: usize = 2048;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PropertyValue {
    Text(String),
    Number(String),
    Boolean(bool),
    Null,
    List(Vec<String>),
}

#[derive(Clone, Debug)]
pub struct Property {
    pub key: String,
    pub key_range: Range<usize>,
    pub value_range: Range<usize>,
    /// Key and value lines, excluding unrelated trailing comments/blank lines.
    pub range: Range<usize>,
    pub value: PropertyValue,
    list_items: Vec<(Range<usize>, Range<usize>)>,
}

impl Property {
    /// The source value span of a simple block-list item. Flow lists have no
    /// independent item spans and remain anchored to the property's value.
    pub fn list_item_range(&self, item: usize) -> Option<Range<usize>> {
        self.list_items.get(item).map(|(_, value)| value.clone())
    }
}

#[derive(Clone, Debug)]
pub struct PropertyFallback {
    pub range: Range<usize>,
    pub reason: String,
}

#[derive(Clone, Debug)]
pub struct Properties {
    pub document_id: DocumentId,
    pub revision: u64,
    pub range: Range<usize>,
    pub content_range: Range<usize>,
    pub entries: Vec<Property>,
    pub fallbacks: Vec<PropertyFallback>,
    eol: String,
    // Includes unsupported/duplicate names so additions cannot shadow them.
    keys: HashSet<String>,
}

#[derive(Clone, Debug)]
pub enum PropertyEdit {
    Rename {
        index: usize,
        key: String,
    },
    SetValue {
        index: usize,
        value: PropertyValue,
    },
    Add {
        key: String,
        value: PropertyValue,
    },
    Delete {
        index: usize,
    },
    SetListItem {
        index: usize,
        item: usize,
        value: String,
    },
    AddListItem {
        index: usize,
        value: String,
    },
    DeleteListItem {
        index: usize,
        item: usize,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub enum PropertyError {
    Stale,
    Invalid(String),
    Document(DocumentError),
}

impl std::fmt::Display for PropertyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stale => f.write_str("Properties changed; refresh before applying this edit"),
            Self::Invalid(reason) => f.write_str(reason),
            Self::Document(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for PropertyError {}

/// A validated draft, bound to one document revision. Does not own edit history.
#[derive(Debug)]
pub struct PropertyTransaction {
    document_id: DocumentId,
    revision: u64,
    range: Range<usize>,
    before: String,
    replacement: String,
}

impl PropertyTransaction {
    /// Returns false for an unchanged draft, without moving selection or adding history.
    pub fn apply(self, document: &mut Document) -> Result<bool, PropertyError> {
        if document.id() != self.document_id
            || document.revision() != self.revision
            || document.source().get(self.range.clone()) != Some(self.before.as_str())
        {
            return Err(PropertyError::Stale);
        }
        if self.before == self.replacement {
            return Ok(false);
        }
        let caret = self.range.start + self.replacement.len();
        document
            .replace_range_with_selection(self.range, &self.replacement, Selection::caret(caret))
            .map_err(PropertyError::Document)?;
        Ok(true)
    }
}

struct Line<'a> {
    start: usize,
    end: usize,
    text: &'a str,
}

impl Properties {
    pub fn parse(document: &Document) -> Option<Self> {
        let source = document.source();
        let range = DisplayProjection::parse(source)
            .blocks
            .into_iter()
            .find(|block| block.kind == BlockKind::FrontMatter)?
            .source;
        let opening_end = source[range.clone()]
            .find('\n')
            .map(|n| range.start + n + 1)?;
        let closing_start = source[..range.end].rfind('\n').map(|n| n + 1)?;
        let content_range = opening_end..closing_start;
        let mut result = Self {
            document_id: document.id(),
            revision: document.revision(),
            range,
            content_range: content_range.clone(),
            entries: Vec::new(),
            fallbacks: Vec::new(),
            eol: if source[..opening_end].ends_with("\r\n") {
                "\r\n"
            } else {
                "\n"
            }
            .into(),
            keys: HashSet::new(),
        };
        if content_range.len() > MAX_BYTES {
            result.fallback(
                content_range,
                "Front matter exceeds the property editor size limit",
            );
            return Some(result);
        }
        let mut offset = opening_end;
        let lines: Vec<_> = source[content_range.clone()]
            .split_inclusive('\n')
            .map(|raw| {
                let line = Line {
                    start: offset,
                    end: offset + raw.len(),
                    text: raw.trim_end_matches(['\r', '\n']),
                };
                offset += raw.len();
                line
            })
            .collect();
        let mut i = 0;
        let mut duplicates = HashSet::new();
        while i < lines.len() {
            let line = &lines[i];
            if line.text.trim().is_empty() || line.text.trim_start().starts_with('#') {
                i += 1;
                continue;
            }
            let sequence_header = key_colon(line.text).is_some_and(|colon| {
                let tail = line.text[colon + 1..].trim_start();
                tail[..comment_start(tail).unwrap_or(tail.len())]
                    .trim()
                    .is_empty()
            });
            let mut end = i + 1;
            while end < lines.len()
                && (lines[end].text.starts_with([' ', '\t'])
                    || lines[end].text.trim().is_empty()
                    || sequence_line(lines[end].text)
                    || (sequence_header
                        && lines[end].text.starts_with('#')
                        && lines[end..]
                            .iter()
                            .find(|line| {
                                !line.text.trim().is_empty()
                                    && !line.text.trim_start().starts_with('#')
                            })
                            .is_some_and(|line| sequence_line(line.text))))
            {
                end += 1;
            }
            // Blank separators do not belong to the preceding property edit.
            let block_scalar = key_colon(line.text)
                .is_some_and(|colon| line.text[colon + 1..].trim_start().starts_with(['|', '>']));
            if !block_scalar {
                while end > i + 1 && lines[end - 1].text.trim().is_empty() {
                    end -= 1;
                }
            }
            let entry_range = line.start..lines[end - 1].end;
            if result.entries.len() + result.fallbacks.len() >= MAX_ENTRIES {
                result.fallback(line.start..content_range.end, "Too many properties");
                break;
            }
            let Some(colon) = key_colon(line.text) else {
                result.fallback(entry_range, "Unsupported or invalid property key");
                i = end;
                continue;
            };
            let key_raw = line.text[..colon].trim_end();
            let Some(key) = parse_key(key_raw) else {
                result.fallback(entry_range, "Unsupported or invalid property key");
                i = end;
                continue;
            };
            if !result.keys.insert(key.clone()) {
                duplicates.insert(key.clone());
            }
            let tail = &line.text[colon + 1..];
            let leading = tail.len() - tail.trim_start_matches(' ').len();
            let raw = &tail[leading..];
            let scalar_end = comment_start(raw).unwrap_or(raw.len());
            let raw = raw[..scalar_end].trim_end();
            let value_start = line.start + colon + 1 + leading;
            let value = if raw.starts_with(['|', '>']) {
                parse_block(raw, &lines[i + 1..end])
            } else if raw.is_empty() && end > i + 1 {
                parse_block_list(&lines[i + 1..end])
            } else if end == i + 1 {
                parse_value(raw)
            } else {
                Err("Nested or multiline plain values require source editing")
            };
            match value {
                Ok(value) => {
                    // Native EDIT is NUL-terminated and keys/items are single-line.
                    // Reject decoded escapes here, before focus can round-trip a
                    // truncated or normalized control value back into the source.
                    let value_supported = match &value {
                        PropertyValue::Text(text) => form_text_supported(text, true),
                        PropertyValue::List(items) => {
                            items.iter().all(|text| form_text_supported(text, false))
                        }
                        _ => true,
                    };
                    if !form_text_supported(&key, false) || !value_supported {
                        result.fallback(entry_range, "Decoded control characters cannot be represented safely in the property form; edit this entry in Source");
                        i = end;
                        continue;
                    }
                    let multiline = end > i + 1 || raw.starts_with(['|', '>']);
                    result.entries.push(Property {
                        key,
                        key_range: line.start..line.start + key_raw.len(),
                        value_range: value_start..if multiline {
                            lines[end - 1].end
                        } else {
                            value_start + raw.len()
                        },
                        range: entry_range,
                        value,
                        list_items: if raw.is_empty() {
                            lines[i + 1..end]
                                .iter()
                                .filter_map(|line| {
                                    let raw = list_item_text(line.text)?;
                                    let prefix_len = line.text.len() - raw.len();
                                    let length = raw[..comment_start(raw).unwrap_or(raw.len())]
                                        .trim_end()
                                        .len();
                                    Some((
                                        line.start..line.end,
                                        line.start + prefix_len..line.start + prefix_len + length,
                                    ))
                                })
                                .collect()
                        } else {
                            Vec::new()
                        },
                    });
                }
                Err(reason) => result.fallback(entry_range, reason),
            }
            i = end;
        }
        // Neither occurrence of a duplicate is safely editable.
        let mut kept = Vec::new();
        for entry in std::mem::take(&mut result.entries) {
            if duplicates.contains(&entry.key) {
                result.fallback(entry.range, "Duplicate property key");
            } else {
                kept.push(entry);
            }
        }
        result.entries = kept;
        result
            .fallbacks
            .sort_by_key(|fallback| fallback.range.start);
        Some(result)
    }

    fn fallback(&mut self, range: Range<usize>, reason: &str) {
        self.fallbacks.push(PropertyFallback {
            range,
            reason: reason.into(),
        });
    }

    pub fn prepare(
        &self,
        document: &Document,
        edit: PropertyEdit,
    ) -> Result<PropertyTransaction, PropertyError> {
        if self.document_id != document.id() || self.revision != document.revision() {
            return Err(PropertyError::Stale);
        }
        let source = document.source();
        let invalid = |reason: &str| PropertyError::Invalid(reason.into());
        let property = |index: usize| {
            self.entries
                .get(index)
                .ok_or_else(|| invalid("Property no longer exists"))
        };
        let (range, replacement) = match edit {
            PropertyEdit::Rename { index, key } => {
                let p = property(index)?;
                validate_key(&key)?;
                if key != p.key && self.keys.contains(&key) {
                    return Err(invalid("A property already has this name"));
                }
                (
                    p.key_range.clone(),
                    if key == p.key {
                        source[p.key_range.clone()].into()
                    } else {
                        quote(&key)
                    },
                )
            }
            PropertyEdit::Add { key, value } => {
                validate_key(&key)?;
                if self.keys.contains(&key) {
                    return Err(invalid("A property already has this name"));
                }
                (
                    self.content_range.end..self.content_range.end,
                    format!(
                        "{}: {}{}",
                        quote(&key),
                        encode(&value, &self.eol)?,
                        self.eol
                    ),
                )
            }
            PropertyEdit::Delete { index } => (property(index)?.range.clone(), String::new()),
            PropertyEdit::SetValue { index, value } => {
                self.value_edit(property(index)?, value, source)?
            }
            PropertyEdit::SetListItem { index, item, value } => {
                let p = property(index)?;
                let mut list = list_value(p)?;
                if let Some((_, range)) = p.list_items.get(item) {
                    let old = list
                        .get(item)
                        .ok_or_else(|| invalid("List item no longer exists"))?;
                    let replacement = if old == &value {
                        source[range.clone()].into()
                    } else {
                        encode(&PropertyValue::Text(value), &self.eol)?
                    };
                    if replacement.contains('\n') {
                        return Err(invalid("List items must be single line text"));
                    }
                    return Ok(PropertyTransaction {
                        document_id: self.document_id,
                        revision: self.revision,
                        before: source[range.clone()].into(),
                        range: range.clone(),
                        replacement,
                    });
                }
                *list
                    .get_mut(item)
                    .ok_or_else(|| invalid("List item no longer exists"))? = value;
                self.value_edit(p, PropertyValue::List(list), source)?
            }
            PropertyEdit::AddListItem { index, value } => {
                let p = property(index)?;
                let mut list = list_value(p)?;
                list.push(value);
                if !p.list_items.is_empty() {
                    let value = encode(&PropertyValue::Text(list.pop().unwrap()), &self.eol)?;
                    if value.contains('\n') {
                        return Err(invalid("List items must be single line text"));
                    }
                    let position = p.range.end;
                    let (item_line, item_value) = p.list_items.last().unwrap();
                    let prefix = &source[item_line.start..item_value.start];
                    return Ok(PropertyTransaction {
                        document_id: self.document_id,
                        revision: self.revision,
                        range: position..position,
                        before: String::new(),
                        replacement: format!("{prefix}{value}{}", self.eol),
                    });
                }
                self.value_edit(p, PropertyValue::List(list), source)?
            }
            PropertyEdit::DeleteListItem { index, item } => {
                let p = property(index)?;
                let mut list = list_value(p)?;
                if item >= list.len() {
                    return Err(invalid("List item no longer exists"));
                }
                list.remove(item);
                if p.list_items.len() > 1 {
                    let range = p.list_items[item].0.clone();
                    return Ok(PropertyTransaction {
                        document_id: self.document_id,
                        revision: self.revision,
                        before: source[range.clone()].into(),
                        range,
                        replacement: String::new(),
                    });
                }
                self.value_edit(p, PropertyValue::List(list), source)?
            }
        };
        Ok(PropertyTransaction {
            document_id: self.document_id,
            revision: self.revision,
            before: source[range.clone()].into(),
            range,
            replacement,
        })
    }

    fn value_edit(
        &self,
        p: &Property,
        value: PropertyValue,
        source: &str,
    ) -> Result<(Range<usize>, String), PropertyError> {
        if p.value == value {
            return Ok((p.value_range.clone(), source[p.value_range.clone()].into()));
        }
        let mut encoded = encode(&value, &self.eol)?;
        let mut range = p.value_range.clone();
        if encoded.contains('\n') && !source[range.clone()].ends_with('\n') {
            let suffix = &source[range.end..];
            let suffix = &suffix[..suffix.find(['\r', '\n']).unwrap_or(suffix.len())];
            if suffix.trim_start().starts_with('#') {
                encoded.insert_str(encoded.find(&self.eol).unwrap(), suffix);
                range.end += suffix.len();
            }
        }
        if let Some(first) = source[p.value_range.clone()].lines().next()
            && let Some(comment) = comment_start(first)
        {
            let comment = &first[comment..];
            if let Some(newline) = encoded.find(&self.eol) {
                encoded.insert_str(newline, &format!(" {comment}"));
            } else {
                encoded.push_str(&format!(" {comment}"));
            }
        }
        // Multiline ranges own their final EOL; inline ranges leave it untouched.
        if source[p.value_range.clone()].ends_with('\n') {
            encoded.push_str(&self.eol);
        }
        if range.start > 0 && source.as_bytes()[range.start - 1] == b':' {
            encoded.insert(0, ' ');
        }
        Ok((range, encoded))
    }
}

fn list_value(p: &Property) -> Result<Vec<String>, PropertyError> {
    if let PropertyValue::List(list) = &p.value {
        Ok(list.clone())
    } else {
        Err(PropertyError::Invalid(
            "This property is not a string list".into(),
        ))
    }
}

fn form_text_supported(text: &str, allow_lf: bool) -> bool {
    !text
        .chars()
        .any(|character| character.is_control() && !(allow_lf && character == '\n'))
}

fn validate_key(key: &str) -> Result<(), PropertyError> {
    if key.is_empty() || key.len() > MAX_BYTES || !form_text_supported(key, false) {
        Err(PropertyError::Invalid(
            "Enter a nonempty property name without control characters".into(),
        ))
    } else {
        Ok(())
    }
}

fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

fn encode(value: &PropertyValue, eol: &str) -> Result<String, PropertyError> {
    let invalid = || PropertyError::Invalid("Value contains unsupported control characters".into());
    let encoded = match value {
        PropertyValue::Null => "null".into(),
        PropertyValue::Boolean(v) => v.to_string(),
        PropertyValue::Number(v) => {
            if !is_number(v) {
                return Err(PropertyError::Invalid(
                    "Enter a valid finite YAML number".into(),
                ));
            }
            v.clone()
        }
        PropertyValue::Text(v) => {
            if !form_text_supported(v, true) {
                return Err(invalid());
            }
            if v.contains('\n') {
                // Chomping preserves the exact number of semantic trailing newlines.
                let trailing = v.len() - v.trim_end_matches('\n').len();
                let header = match trailing {
                    0 => "|2-",
                    1 => "|2",
                    _ => "|2+",
                };
                let body = v.strip_suffix('\n').unwrap_or(v);
                format!(
                    "{header}{eol}{}",
                    body.split('\n')
                        .map(|line| format!("  {line}"))
                        .collect::<Vec<_>>()
                        .join(eol)
                )
            } else {
                quote(v)
            }
        }
        PropertyValue::List(values) => {
            if values.iter().any(|v| !form_text_supported(v, false)) {
                return Err(invalid());
            }
            format!(
                "[{}]",
                values
                    .iter()
                    .map(|v| quote(v))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    };
    if encoded.len() > MAX_BYTES {
        return Err(PropertyError::Invalid(
            "Value exceeds the property editor size limit".into(),
        ));
    }
    Ok(encoded)
}

fn is_number(raw: &str) -> bool {
    !raw.is_empty()
        && raw.trim() == raw
        && !raw.contains(['_', 'x', 'o'])
        && raw
            .bytes()
            .all(|c| c.is_ascii_digit() || b"-+.eE".contains(&c))
        && raw.parse::<f64>().is_ok_and(f64::is_finite)
}

fn key_colon(text: &str) -> Option<usize> {
    if text.starts_with([' ', '\t', '#', '?', '-']) {
        return None;
    }
    delimiters(text, ':')
        .ok()?
        .into_iter()
        .find(|&i| text[i + 1..].is_empty() || text.as_bytes()[i + 1].is_ascii_whitespace())
}

fn parse_key(raw: &str) -> Option<String> {
    if raw.starts_with(['\'', '"']) {
        return parse_quoted(raw).ok();
    }
    (!raw.is_empty()
        && !raw.contains(['[', ']', '{', '}', '&', '*', '!', '#', ',', '\t'])
        && !raw.chars().any(char::is_control))
    .then(|| raw.into())
}

// Scan syntax delimiters only outside quoted strings; reject unclosed quotes.
fn delimiters(raw: &str, delimiter: char) -> Result<Vec<usize>, &'static str> {
    let mut result = Vec::new();
    let mut quote = None;
    let mut chars = raw.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match quote {
            Some('"') if c == '\\' => {
                chars.next();
            }
            Some('\'') if c == '\'' && chars.peek().is_some_and(|(_, next)| *next == '\'') => {
                chars.next();
            }
            Some(q) if c == q => quote = None,
            None if (c == '\'' || c == '"')
                && (raw[..i].trim().is_empty()
                    || raw[..i].trim_end().ends_with(['[', ',', ':'])) =>
            {
                quote = Some(c)
            }
            None if c == delimiter => result.push(i),
            _ => {}
        }
    }
    if quote.is_some() {
        Err("Unclosed quoted scalar")
    } else {
        Ok(result)
    }
}

fn comment_start(raw: &str) -> Option<usize> {
    delimiters(raw, '#')
        .ok()?
        .into_iter()
        .find(|&i| i == 0 || raw.as_bytes()[i - 1].is_ascii_whitespace())
}

fn parse_quoted(raw: &str) -> Result<String, &'static str> {
    if raw.starts_with('\'') {
        if !raw.ends_with('\'') || raw.len() < 2 {
            return Err("Unclosed quoted scalar");
        }
        let inner = &raw[1..raw.len() - 1];
        let mut chars = inner.chars();
        let mut out = String::new();
        while let Some(c) = chars.next() {
            if c == '\'' && chars.next() != Some('\'') {
                return Err("Invalid single quoted scalar");
            }
            out.push(c);
        }
        Ok(out)
    } else {
        // JSON's escape subset is valid YAML. Other YAML escapes stay in explicit fallback.
        serde_json::from_str::<String>(raw)
            .map_err(|_| "Unsupported or invalid quoted scalar escape")
    }
}

fn parse_value(raw: &str) -> Result<PropertyValue, &'static str> {
    if raw.starts_with(['\'', '"']) {
        return parse_quoted(raw).map(PropertyValue::Text);
    }
    if raw.starts_with('[') {
        if !raw.ends_with(']') {
            return Err("Unclosed flow list");
        }
        let inner = &raw[1..raw.len() - 1];
        if inner.trim().is_empty() {
            return Ok(PropertyValue::List(Vec::new()));
        }
        let mut bounds = delimiters(inner, ',')?;
        bounds.push(inner.len());
        let mut start = 0;
        let mut items = Vec::new();
        for end in bounds {
            let item = inner[start..end].trim();
            start = end + 1;
            if item.starts_with('[') {
                return Err("Nested lists require source editing");
            }
            match parse_value(item)? {
                PropertyValue::Text(text) => items.push(text),
                _ => return Err("Only string list items are supported"),
            }
        }
        return Ok(PropertyValue::List(items));
    }
    if raw.starts_with(['{', '&', '*', '!', '|', '>', '@', '`', ']', '}'])
        || matches!(
            raw,
            ".nan" | ".NaN" | ".NAN" | ".inf" | ".Inf" | ".INF" | "-.inf" | "+.inf"
        )
        || raw.starts_with("0x")
        || raw.starts_with("0o")
        || raw.starts_with("0b")
        || raw.contains(": ")
        || raw.starts_with("- ")
        || raw.starts_with("? ")
        || raw.contains(['[', ']', '{', '}'])
        || raw.chars().any(char::is_control)
    {
        return Err(
            "Nested structures, references, tags or invalid scalars require source editing",
        );
    }
    Ok(match raw {
        "" | "null" | "Null" | "NULL" | "~" => PropertyValue::Null,
        "true" | "True" | "TRUE" => PropertyValue::Boolean(true),
        "false" | "False" | "FALSE" => PropertyValue::Boolean(false),
        _ if is_number(raw) => PropertyValue::Number(raw.into()),
        _ => PropertyValue::Text(raw.into()),
    })
}

fn sequence_line(text: &str) -> bool {
    let text = text.trim_start_matches(' ');
    text == "-" || text.starts_with("- ")
}

fn list_item_text(text: &str) -> Option<&str> {
    text.trim_start_matches(' ').strip_prefix("- ")
}

fn parse_block_list(lines: &[Line<'_>]) -> Result<PropertyValue, &'static str> {
    let mut items = Vec::new();
    let mut indent = None;
    for line in lines {
        if line.text.trim().is_empty() || line.text.trim_start().starts_with('#') {
            continue;
        }
        let Some(raw) = list_item_text(line.text) else {
            return Err("Nested or complex list requires source editing");
        };
        let item_indent = line.text.len() - line.text.trim_start_matches(' ').len();
        if indent.is_some_and(|indent| indent != item_indent) {
            return Err("Nested or inconsistently indented list requires source editing");
        }
        indent = Some(item_indent);
        let end = comment_start(raw).unwrap_or(raw.len());
        match parse_value(raw[..end].trim_end())? {
            PropertyValue::Text(text) => items.push(text),
            _ => return Err("Only string list items are supported"),
        }
    }
    Ok(PropertyValue::List(items))
}

fn parse_block(header: &str, lines: &[Line<'_>]) -> Result<PropertyValue, &'static str> {
    if !matches!(
        header,
        "|" | "|-" | "|+" | "|2" | "|2-" | "|2+" | ">" | ">-" | ">+"
    ) {
        return Err("Unsupported block scalar indentation or chomping header");
    }
    let indent = if header.contains('2') {
        2
    } else {
        lines
            .iter()
            .find(|line| !line.text.trim().is_empty())
            .map(|line| line.text.len() - line.text.trim_start_matches(' ').len())
            .unwrap_or(2)
    };
    if indent == 0 {
        return Err("Invalid block scalar indentation");
    }
    let mut values = Vec::new();
    for line in lines {
        if line.text.trim().is_empty() {
            values.push(if line.text.len() > indent {
                &line.text[indent..]
            } else {
                ""
            });
        } else if line.text.len() >= indent
            && line.text.as_bytes()[..indent].iter().all(|c| *c == b' ')
        {
            values.push(&line.text[indent..]);
        } else {
            return Err("Invalid block scalar indentation");
        }
    }
    if header.starts_with('>')
        && values
            .iter()
            .any(|value| value.is_empty() || value.starts_with(' '))
    {
        return Err("Folded scalars with blank or more indented lines require source editing");
    }
    let mut text = String::new();
    for (i, value) in values.iter().enumerate() {
        text.push_str(value);
        if i + 1 < values.len() {
            let next = values[i + 1];
            text.push(
                if header.starts_with('>')
                    && !value.is_empty()
                    && !next.is_empty()
                    && !value.starts_with(' ')
                    && !next.starts_with(' ')
                {
                    ' '
                } else {
                    '\n'
                },
            );
        } else {
            text.push('\n');
        }
    }
    if header.ends_with('-') {
        text.truncate(text.trim_end_matches('\n').len());
    } else if !header.ends_with('+') && !text.is_empty() {
        text.truncate(text.trim_end_matches('\n').len());
        text.push('\n');
    }
    Ok(PropertyValue::Text(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn document(source: &str) -> Document {
        Document::new(7, source.into())
    }
    fn edit(doc: &mut Document, edit: PropertyEdit) {
        Properties::parse(doc)
            .unwrap()
            .prepare(doc, edit)
            .unwrap()
            .apply(doc)
            .unwrap();
    }
    fn set(doc: &mut Document, index: usize, value: PropertyValue) {
        edit(doc, PropertyEdit::SetValue { index, value });
    }

    #[test]
    fn typed_projection_never_changes_source_or_history() {
        let original = "---\nnullish: null\nempty: ''\nflag: false\ncount: 0\nquoted: \"001\"\ndate: 2026-10-23\nurl: https://example.invalid/a#part\n---\nBody\n";
        let doc = document(original);
        let p = Properties::parse(&doc).unwrap();
        assert!(p.fallbacks.is_empty(), "{:?}", p.fallbacks);
        assert_eq!(
            p.entries
                .iter()
                .map(|p| p.value.clone())
                .collect::<Vec<_>>(),
            vec![
                PropertyValue::Null,
                PropertyValue::Text("".into()),
                PropertyValue::Boolean(false),
                PropertyValue::Number("0".into()),
                PropertyValue::Text("001".into()),
                PropertyValue::Text("2026-10-23".into()),
                PropertyValue::Text("https://example.invalid/a#part".into())
            ]
        );
        assert_eq!(doc.source(), original);
        assert_eq!(doc.revision(), 0);
        assert!(!doc.is_dirty());
        assert_eq!(doc.undo_count(), 0);
    }

    #[test]
    fn crlf_targeted_edit_and_history_restore_literal_bytes_and_selection() {
        let original = "---\r\n# keep\r\nname: \"001\" # comment\r\nunknown: !tag preserved\r\n---\r\n\r\nBody | untouched\r\n";
        let mut doc = document(original);
        doc.set_selection(4, 4).unwrap();
        set(&mut doc, 0, PropertyValue::Text("002".into()));
        let expected = "---\r\n# keep\r\nname: '002' # comment\r\nunknown: !tag preserved\r\n---\r\n\r\nBody | untouched\r\n";
        assert_eq!(doc.source(), expected);
        assert_eq!(doc.undo_count(), 1);
        let selection = doc.selection();
        assert!(doc.undo());
        assert_eq!(doc.source(), original);
        assert_eq!(doc.selection(), Selection::caret(4));
        assert!(!doc.is_dirty());
        assert!(doc.redo());
        assert_eq!(doc.source(), expected);
        assert_eq!(doc.selection(), selection);
    }

    #[test]
    fn rename_add_delete_are_individual_source_transactions() {
        let mut doc = document("---\na: true\n# stays\nb: 2\n---\nBody");
        edit(
            &mut doc,
            PropertyEdit::Rename {
                index: 0,
                key: "renamed".into(),
            },
        );
        assert_eq!(
            doc.source(),
            "---\n'renamed': true\n# stays\nb: 2\n---\nBody"
        );
        edit(
            &mut doc,
            PropertyEdit::Add {
                key: "empty".into(),
                value: PropertyValue::Text("".into()),
            },
        );
        assert_eq!(
            doc.source(),
            "---\n'renamed': true\n# stays\nb: 2\n'empty': ''\n---\nBody"
        );
        edit(&mut doc, PropertyEdit::Delete { index: 1 });
        assert_eq!(
            doc.source(),
            "---\n'renamed': true\n# stays\n'empty': ''\n---\nBody"
        );
        assert_eq!(doc.undo_count(), 3);
    }

    #[test]
    fn stale_document_revision_invalid_drafts_and_noop_leave_bytes_and_history() {
        let mut doc = document("---\na: false\n---\n");
        let p = Properties::parse(&doc).unwrap();
        let same = p
            .prepare(
                &doc,
                PropertyEdit::SetValue {
                    index: 0,
                    value: PropertyValue::Boolean(false),
                },
            )
            .unwrap();
        assert!(!same.apply(&mut doc).unwrap());
        assert_eq!(doc.undo_count(), 0);
        assert_eq!(doc.revision(), 0);
        assert!(!doc.is_dirty());
        assert!(
            p.prepare(
                &doc,
                PropertyEdit::SetValue {
                    index: 0,
                    value: PropertyValue::Number("oops".into())
                }
            )
            .is_err()
        );
        assert!(
            p.prepare(
                &doc,
                PropertyEdit::Add {
                    key: "a".into(),
                    value: PropertyValue::Null
                }
            )
            .is_err()
        );
        let tx = p.prepare(&doc, PropertyEdit::Delete { index: 0 }).unwrap();
        let mut other = Document::new(8, doc.source().into());
        assert_eq!(tx.apply(&mut other), Err(PropertyError::Stale));
        let tx = p.prepare(&doc, PropertyEdit::Delete { index: 0 }).unwrap();
        doc.replace_range(7..12, "true").unwrap();
        assert_eq!(tx.apply(&mut doc), Err(PropertyError::Stale));
        assert!(matches!(
            p.prepare(&doc, PropertyEdit::Delete { index: 0 }),
            Err(PropertyError::Stale)
        ));
    }

    #[test]
    fn block_lists_edit_items_without_touching_other_items_or_comments() {
        let mut doc =
            document("---\nauthor:\n  - 'A' # first\n  - \"B\" # second\nkeep: x\n---\nBody");
        edit(
            &mut doc,
            PropertyEdit::SetListItem {
                index: 0,
                item: 0,
                value: "C".into(),
            },
        );
        assert_eq!(
            doc.source(),
            "---\nauthor:\n  - 'C' # first\n  - \"B\" # second\nkeep: x\n---\nBody"
        );
        edit(
            &mut doc,
            PropertyEdit::AddListItem {
                index: 0,
                value: "D".into(),
            },
        );
        assert_eq!(
            doc.source(),
            "---\nauthor:\n  - 'C' # first\n  - \"B\" # second\n  - 'D'\nkeep: x\n---\nBody"
        );
        edit(&mut doc, PropertyEdit::DeleteListItem { index: 0, item: 1 });
        assert_eq!(
            doc.source(),
            "---\nauthor:\n  - 'C' # first\n  - 'D'\nkeep: x\n---\nBody"
        );
    }

    #[test]
    fn indentless_lists_own_the_sequence_and_preserve_item_style_comments_and_history() {
        for eol in ["\n", "\r\n"] {
            let original="---\ntags: # header\n- one # first\n# between\n- 'two' # second\n# next property\nempty:\nnext: untouched\n---\nBody\n".replace('\n',eol);
            let mut doc = document(&original);
            let p = Properties::parse(&doc).unwrap();
            assert!(p.fallbacks.is_empty(), "{:?}", p.fallbacks);
            assert_eq!(
                p.entries[0].value,
                PropertyValue::List(vec!["one".into(), "two".into()])
            );
            assert_eq!(p.entries[1].value, PropertyValue::Null);
            assert_eq!(
                &doc.source()[p.entries[0].list_item_range(1).unwrap()],
                "'two'"
            );
            assert_eq!(
                &doc.source()[p.entries[0].range.clone()],
                "tags: # header\n- one # first\n# between\n- 'two' # second\n".replace('\n', eol)
            );
            doc.set_selection(4, 4).unwrap();
            edit(
                &mut doc,
                PropertyEdit::SetListItem {
                    index: 0,
                    item: 1,
                    value: "changed".into(),
                },
            );
            let expected="---\ntags: # header\n- one # first\n# between\n- 'changed' # second\n# next property\nempty:\nnext: untouched\n---\nBody\n".replace('\n',eol);
            assert_eq!(doc.source(), expected);
            assert_eq!(doc.undo_count(), 1);
            assert!(doc.undo());
            assert_eq!(doc.source(), original);
            assert_eq!(doc.selection(), Selection::caret(4));
            assert!(!doc.is_dirty());
            assert!(doc.redo());
            assert_eq!(doc.source(), expected);
            edit(
                &mut doc,
                PropertyEdit::AddListItem {
                    index: 0,
                    value: "three".into(),
                },
            );
            let expected="---\ntags: # header\n- one # first\n# between\n- 'changed' # second\n- 'three'\n# next property\nempty:\nnext: untouched\n---\nBody\n".replace('\n',eol);
            assert_eq!(doc.source(), expected);
            edit(&mut doc, PropertyEdit::DeleteListItem { index: 0, item: 0 });
            assert_eq!(doc.source(),"---\ntags: # header\n# between\n- 'changed' # second\n- 'three'\n# next property\nempty:\nnext: untouched\n---\nBody\n".replace('\n',eol));
        }
    }

    #[test]
    fn replacing_indentless_list_cannot_leave_orphan_sequence_lines() {
        for eol in ["\n", "\r\n"] {
            let original =
                "---\ntags:\n- one\n- two\n# keep boundary\nnext: 0\n---\nBody".replace('\n', eol);
            let mut doc = document(&original);
            set(&mut doc, 0, PropertyValue::List(vec!["new".into()]));
            let expected =
                "---\ntags: ['new']\n# keep boundary\nnext: 0\n---\nBody".replace('\n', eol);
            assert_eq!(doc.source(), expected);
            assert_eq!(doc.undo_count(), 1);
            let p = Properties::parse(&doc).unwrap();
            assert!(p.fallbacks.is_empty());
            assert_eq!(p.entries[0].value, PropertyValue::List(vec!["new".into()]));
            assert!(doc.undo());
            assert_eq!(doc.source(), original);
            assert!(doc.redo());
            assert_eq!(doc.source(), expected);
        }
    }

    #[test]
    fn unsupported_indentless_sequence_is_one_complete_fallback_not_editable_null() {
        let original = "---\ntags:\n- one\n- 0\nnext: null\n---\nBody";
        let doc = document(original);
        let p = Properties::parse(&doc).unwrap();
        assert_eq!(p.entries.len(), 1);
        assert_eq!(p.entries[0].key, "next");
        assert_eq!(p.entries[0].value, PropertyValue::Null);
        assert_eq!(p.fallbacks.len(), 1);
        assert_eq!(
            &doc.source()[p.fallbacks[0].range.clone()],
            "tags:\n- one\n- 0\n"
        );
        assert_eq!(doc.source(), original);
        assert_eq!(doc.revision(), 0);
        assert_eq!(doc.undo_count(), 0);
        let mut indented = document("---\ntags:\n    - one\n    - two\nnext: x\n---\n");
        edit(
            &mut indented,
            PropertyEdit::AddListItem {
                index: 0,
                value: "three".into(),
            },
        );
        assert_eq!(
            indented.source(),
            "---\ntags:\n    - one\n    - two\n    - 'three'\nnext: x\n---\n"
        );
    }

    #[test]
    fn flow_lists_quotes_unicode_and_last_item_removal() {
        let mut doc = document("---\ntags: [\"a,b\", 'it''s', 日本語] # keep\n---\n");
        let p = Properties::parse(&doc).unwrap();
        assert_eq!(
            p.entries[0].value,
            PropertyValue::List(vec!["a,b".into(), "it's".into(), "日本語".into()])
        );
        edit(
            &mut doc,
            PropertyEdit::SetListItem {
                index: 0,
                item: 1,
                value: "x:y".into(),
            },
        );
        assert_eq!(
            doc.source(),
            "---\ntags: ['a,b', 'x:y', '日本語'] # keep\n---\n"
        );
        let mut doc = document("---\na:\n  - only\n---\n");
        edit(&mut doc, PropertyEdit::DeleteListItem { index: 0, item: 0 });
        assert_eq!(doc.source(), "---\na: []\n---\n");
    }

    #[test]
    fn multiline_literal_folded_chomping_and_edited_text_roundtrip() {
        let doc =
            document("---\nliteral: |-\n  A\n  B\nfolded: >-\n  A\n  B\nkeep: |+\n  A\n\n---\n");
        let p = Properties::parse(&doc).unwrap();
        assert_eq!(p.entries[0].value, PropertyValue::Text("A\nB".into()));
        assert_eq!(p.entries[1].value, PropertyValue::Text("A B".into()));
        assert_eq!(p.entries[2].value, PropertyValue::Text("A\n\n".into()));
        for value in ["A\nB", "A\nB\n", "A\nB\n\n", "\n", "  indented\n B\n"] {
            let mut doc = document("---\na: old # keep\n---\nBody");
            set(&mut doc, 0, PropertyValue::Text(value.into()));
            let p = Properties::parse(&doc).unwrap();
            assert!(p.fallbacks.is_empty(), "{}", doc.source());
            assert_eq!(
                p.entries[0].value,
                PropertyValue::Text(value.into()),
                "{}",
                doc.source()
            );
            assert!(doc.source().ends_with("---\nBody"));
            assert!(doc.source().contains("# keep"));
        }
    }

    #[test]
    fn unsupported_ranges_and_duplicate_keys_are_explicit_and_preserved() {
        let original = "---\nnested:\n  a: 1\nbase: &base {key: value}\ncopy: *base\ncustom: !unknown keep\nduplicate: first\nduplicate: second\ngood: yes\nbad: \"unterminated\n---\nBody";
        let mut doc = document(original);
        let p = Properties::parse(&doc).unwrap();
        assert_eq!(p.entries.len(), 1);
        assert_eq!(p.entries[0].key, "good");
        assert_eq!(p.fallbacks.len(), 7);
        for fallback in &p.fallbacks {
            assert!(!fallback.reason.is_empty());
            assert!(!doc.source()[fallback.range.clone()].is_empty());
        }
        assert!(
            p.prepare(
                &doc,
                PropertyEdit::Add {
                    key: "base".into(),
                    value: PropertyValue::Null
                }
            )
            .is_err()
        );
        set(&mut doc, 0, PropertyValue::Text("changed".into()));
        assert_eq!(
            doc.source(),
            original.replace("good: yes", "good: 'changed'")
        );
    }

    #[test]
    fn decoded_controls_fallback_complete_entries_without_source_or_history_changes() {
        for eol in ["\n", "\r\n"] {
            for escape in [r"\u0000", r"\r", r"\t", r"\b", r"\f", r"\u007f"] {
                let unsafe_entries = [
                    format!("title: \"abc{escape}def\" # preserve scalar\n"),
                    format!("\"abc{escape}def\": value # preserve key\n"),
                    format!("tags: [\"abc{escape}def\", 'safe item'] # preserve flow list\n"),
                    format!(
                        "tags: # preserve block list\n- 'safe item'\n- \"abc{escape}def\" # preserve item\n"
                    ),
                ];
                for entry in unsafe_entries {
                    let entry = entry.replace('\n', eol);
                    let original = format!(
                        "---{eol}# before{eol}{entry}# boundary{eol}safe: untouched{eol}---{eol}Body{eol}"
                    );
                    let mut doc = document(&original);
                    let p = Properties::parse(&doc).unwrap();
                    assert_eq!(p.entries.len(), 1, "{original}");
                    assert_eq!(p.entries[0].key, "safe");
                    assert_eq!(p.fallbacks.len(), 1, "{original}");
                    assert!(p.fallbacks[0].reason.contains("Decoded control characters"));
                    assert_eq!(&doc.source()[p.fallbacks[0].range.clone()], entry);
                    assert_eq!(doc.source(), original);
                    assert_eq!(doc.revision(), 0);
                    assert!(!doc.is_dirty());
                    assert_eq!(doc.undo_count(), 0);
                    assert!(
                        !p.prepare(
                            &doc,
                            PropertyEdit::SetValue {
                                index: 0,
                                value: PropertyValue::Text("untouched".into())
                            }
                        )
                        .unwrap()
                        .apply(&mut doc)
                        .unwrap()
                    );
                    assert_eq!(doc.revision(), 0);
                    assert!(!doc.is_dirty());
                    assert_eq!(doc.undo_count(), 0);
                    set(&mut doc, 0, PropertyValue::Text("changed".into()));
                    assert_eq!(
                        doc.source(),
                        format!(
                            "---{eol}# before{eol}{entry}# boundary{eol}safe: 'changed'{eol}---{eol}Body{eol}"
                        )
                    );
                    assert_eq!(doc.undo_count(), 1);
                    assert!(doc.undo());
                    assert_eq!(doc.source(), original);
                    assert!(!doc.is_dirty());
                }
            }
        }
    }

    #[test]
    fn decoded_lf_is_text_multiline_but_not_a_key_or_list_item() {
        for eol in ["\n", "\r\n"] {
            let original = concat!(
                "---\n",
                r#"title: "line\nnext" # valid multiline text"#,
                "\n",
                r#""bad\nkey": value"#,
                "\n",
                r#"flow: ["bad\nitem"]"#,
                "\n",
                "block:\n  - \"bad\\nitem\"\n",
                r#"literal: 'abc\ndef'"#,
                "\n---\nBody"
            )
            .replace('\n', eol);
            let doc = document(&original);
            let p = Properties::parse(&doc).unwrap();
            assert_eq!(p.entries.len(), 2);
            assert_eq!(p.fallbacks.len(), 3);
            assert_eq!(p.entries[0].value, PropertyValue::Text("line\nnext".into()));
            assert_eq!(p.entries[1].value, PropertyValue::Text(r"abc\ndef".into()));
            assert_eq!(doc.source(), original);
            assert_eq!(doc.revision(), 0);
            assert_eq!(doc.undo_count(), 0);
            assert!(!doc.is_dirty());
        }
    }

    #[test]
    fn ordinary_unicode_flat_values_stay_editable_and_control_drafts_stay_invalid() {
        let original = concat!(
            "---\n",
            r#""名前😀": "日本語 👩‍💻 é""#,
            "\n",
            r#"date: "2026-10-23T12:30:00+09:00""#,
            "\n",
            "url: https://example.invalid/a#part\n",
            r#"numeric: "001""#,
            "\n",
            r#"tags: ["日本語", "emoji 😀"]"#,
            "\n---\nBody"
        );
        let doc = document(original);
        let p = Properties::parse(&doc).unwrap();
        assert!(p.fallbacks.is_empty());
        assert_eq!(p.entries.len(), 5);
        assert_eq!(p.entries[0].key, "名前😀");
        assert_eq!(
            p.entries[0].value,
            PropertyValue::Text("日本語 👩‍💻 é".into())
        );
        assert_eq!(
            p.entries[1].value,
            PropertyValue::Text("2026-10-23T12:30:00+09:00".into())
        );
        assert_eq!(
            p.entries[2].value,
            PropertyValue::Text("https://example.invalid/a#part".into())
        );
        assert_eq!(p.entries[3].value, PropertyValue::Text("001".into()));
        for control in ['\0', '\r', '\t', '\u{7f}'] {
            let text = format!("abc{control}def");
            assert!(
                p.prepare(
                    &doc,
                    PropertyEdit::Rename {
                        index: 0,
                        key: text.clone()
                    }
                )
                .is_err()
            );
            assert!(
                p.prepare(
                    &doc,
                    PropertyEdit::SetValue {
                        index: 0,
                        value: PropertyValue::Text(text.clone())
                    }
                )
                .is_err()
            );
            assert!(
                p.prepare(
                    &doc,
                    PropertyEdit::SetValue {
                        index: 4,
                        value: PropertyValue::List(vec![text])
                    }
                )
                .is_err()
            );
        }
        assert_eq!(doc.source(), original);
        assert_eq!(doc.revision(), 0);
        assert!(!doc.is_dirty());
        assert_eq!(doc.undo_count(), 0);
    }

    #[test]
    fn only_recognized_front_matter_and_empty_block_are_projected() {
        assert!(Properties::parse(&document("text\n---\na: b\n---\n")).is_none());
        assert!(Properties::parse(&document("---\na: b\n")).is_none());
        let mut doc = document("---\n---\nBody");
        edit(
            &mut doc,
            PropertyEdit::Add {
                key: "a".into(),
                value: PropertyValue::Null,
            },
        );
        assert_eq!(doc.source(), "---\n'a': null\n---\nBody");
    }
}
