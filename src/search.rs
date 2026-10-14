//! Bounded PCRE2 search over immutable UTF-8 document snapshots.
use crate::document::{Document, Selection};
use pcre2_sys::*;
use std::{
    ffi::{c_int, c_void},
    ops::Range,
    ptr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    pub text: String,
    pub case_sensitive: bool,
    pub regex: bool,
    pub word: bool,
}

/// Regex source is capped at 4096 UTF-16 code units, matching the legacy
/// contract. Literal source is capped by the scanner's 8 MiB per-file budget;
/// escaping it for PCRE2 can at most double that input plus a small word-boundary
/// wrapper. Both modes retain PCRE2's 200 ms match deadline and match/depth/heap
/// limits; compile, allocation, and resource failures stay errors, not zero hits.
pub const MAX_REGEX_QUERY_UTF16: usize = 4096;
pub const MAX_LITERAL_QUERY_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchState {
    Complete,
    Cancelled,
    TimedOut,
    ResourceLimit,
    HitLimit,
}
#[derive(Clone, Debug)]
pub struct Hit {
    pub range: Range<usize>,
    pub captures: Vec<Option<Range<usize>>>,
}
pub struct Results {
    pub document_id: u64,
    pub revision: u64,
    pub generation: u64,
    pub source: Arc<str>,
    pub query: Query,
    pub hits: Vec<Hit>,
    pub state: SearchState,
}
#[derive(Debug)]
pub enum SearchError {
    Empty,
    PatternTooLong,
    QueryTooLong,
    Compile { code: i32, offset: usize },
    Engine(i32),
    Allocation,
    Stale,
    Incomplete,
    ReplacementTooLarge,
    InvalidHit,
}
impl std::fmt::Display for SearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PatternTooLong => {
                f.write_str("regular-expression query exceeds 4096 UTF-16 code units")
            }
            Self::QueryTooLong => f.write_str("literal query exceeds the 8 MiB UTF-8 input limit"),
            _ => write!(f, "{self:?}"),
        }
    }
}
impl std::error::Error for SearchError {}
const MAX_HITS: usize = 20_000;
const MAX_RESULT_BYTES: usize = 8 * 1024 * 1024;
const MAX_PLAN_BYTES: usize = 64 * 1024 * 1024;

// The upstream C API exports callouts; pcre2-sys omits this declaration. The
// opaque block is intentionally never dereferenced. PCRE2 invokes synchronously.
unsafe extern "C" {
    fn pcre2_set_callout_8(
        context: *mut pcre2_match_context_8,
        callout: Option<unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int>,
        data: *mut c_void,
    ) -> c_int;
}
struct Budget<'a> {
    cancel: &'a AtomicBool,
    deadline: Instant,
}
unsafe extern "C" fn callout(_: *mut c_void, data: *mut c_void) -> c_int {
    // SAFETY: search retains this stack Budget until synchronous matching ends.
    let budget = unsafe { &*(data as *const Budget<'_>) };
    if budget.cancel.load(Ordering::Relaxed) || Instant::now() >= budget.deadline {
        PCRE2_ERROR_CALLOUT
    } else {
        0
    }
}
struct Engine {
    code: *mut pcre2_code_8,
    data: *mut pcre2_match_data_8,
    context: *mut pcre2_match_context_8,
}
impl Drop for Engine {
    fn drop(&mut self) {
        unsafe {
            if !self.context.is_null() {
                pcre2_match_context_free_8(self.context);
            }
            if !self.data.is_null() {
                pcre2_match_data_free_8(self.data);
            }
            if !self.code.is_null() {
                pcre2_code_free_8(self.code);
            }
        }
    }
}
impl Engine {
    fn compile(query: &Query) -> Result<Self, SearchError> {
        if query.text.is_empty() {
            return Err(SearchError::Empty);
        }
        if query.regex {
            if query.text.encode_utf16().count() > MAX_REGEX_QUERY_UTF16 {
                return Err(SearchError::PatternTooLong);
            }
        } else if query.text.len() > MAX_LITERAL_QUERY_BYTES {
            return Err(SearchError::QueryTooLong);
        }
        let mut pattern = if query.regex {
            query.text.clone()
        } else {
            escape_literal(&query.text)
        };
        if query.word {
            pattern = format!("(?<!\\w)(?:{pattern})(?!\\w)");
        }
        let mut error = 0;
        let mut offset = 0;
        let mut options = PCRE2_UTF
            | PCRE2_UCP
            | PCRE2_ALT_BSUX
            | PCRE2_MATCH_UNSET_BACKREF
            | PCRE2_NEVER_BACKSLASH_C
            | PCRE2_AUTO_CALLOUT
            | PCRE2_NO_START_OPTIMIZE;
        if !query.case_sensitive {
            options |= PCRE2_CASELESS;
        }
        // SAFETY: all pointers refer to live buffers; allocations are freed by Engine.
        unsafe {
            let code = pcre2_compile_8(
                pattern.as_ptr(),
                pattern.len(),
                options,
                &mut error,
                &mut offset,
                ptr::null_mut(),
            );
            if code.is_null() {
                return Err(SearchError::Compile {
                    code: error,
                    offset,
                });
            }
            let engine = Self {
                code,
                data: pcre2_match_data_create_from_pattern_8(code, ptr::null_mut()),
                context: pcre2_match_context_create_8(ptr::null_mut()),
            };
            if engine.data.is_null() || engine.context.is_null() {
                return Err(SearchError::Allocation);
            }
            pcre2_set_match_limit_8(engine.context, 1_000_000);
            pcre2_set_depth_limit_8(engine.context, 512);
            pcre2_set_heap_limit_8(engine.context, 8 * 1024);
            Ok(engine)
        }
    }
}

fn escape_literal(text: &str) -> String {
    let mut pattern = String::with_capacity(text.len());
    for character in text.chars() {
        if "\\.^$|?*+()[]{}".contains(character) {
            pattern.push('\\');
        }
        pattern.push(character);
    }
    pattern
}

pub fn search_document(
    doc: &Document,
    query: Query,
    generation: u64,
    cancel: &AtomicBool,
) -> Result<Results, SearchError> {
    search_snapshot(
        doc.id(),
        doc.revision(),
        doc.source_shared(),
        query,
        generation,
        cancel,
        Duration::from_millis(200),
    )
}
pub fn search_snapshot(
    document_id: u64,
    revision: u64,
    source: Arc<str>,
    query: Query,
    generation: u64,
    cancel: &AtomicBool,
    timeout: Duration,
) -> Result<Results, SearchError> {
    let engine = Engine::compile(&query)?;
    let mut budget = Budget {
        cancel,
        deadline: Instant::now() + timeout.min(Duration::from_millis(200)),
    };
    // SAFETY: Budget is stable until engine matching has stopped and never crosses threads.
    unsafe {
        pcre2_set_callout_8(
            engine.context,
            Some(callout),
            (&mut budget as *mut Budget<'_>).cast(),
        );
    }
    let mut result = Results {
        document_id,
        revision,
        generation,
        source,
        query,
        hits: Vec::new(),
        state: SearchState::Complete,
    };
    let mut offset = 0;
    let mut retained = 0usize;
    while offset <= result.source.len() {
        if cancel.load(Ordering::Relaxed) {
            result.state = SearchState::Cancelled;
            break;
        }
        if Instant::now() >= budget.deadline {
            result.state = SearchState::TimedOut;
            break;
        }
        let count = unsafe {
            pcre2_match_8(
                engine.code,
                result.source.as_ptr(),
                result.source.len(),
                offset,
                0,
                engine.data,
                engine.context,
            )
        };
        if count == PCRE2_ERROR_NOMATCH {
            break;
        }
        if count == PCRE2_ERROR_CALLOUT {
            result.state = if cancel.load(Ordering::Relaxed) {
                SearchState::Cancelled
            } else {
                SearchState::TimedOut
            };
            break;
        }
        if [
            PCRE2_ERROR_MATCHLIMIT,
            PCRE2_ERROR_DEPTHLIMIT,
            PCRE2_ERROR_HEAPLIMIT,
        ]
        .contains(&count)
        {
            result.state = SearchState::ResourceLimit;
            break;
        }
        if count < 0 {
            return Err(SearchError::Engine(count));
        }
        let capture_count = unsafe { pcre2_get_ovector_count_8(engine.data) } as usize;
        let offsets = unsafe {
            std::slice::from_raw_parts(pcre2_get_ovector_pointer_8(engine.data), capture_count * 2)
        };
        let mut captures = Vec::with_capacity(capture_count);
        for pair in offsets.as_chunks::<2>().0 {
            if pair[0] == usize::MAX {
                captures.push(None);
            } else {
                if pair[0] > pair[1]
                    || !result.source.is_char_boundary(pair[0])
                    || !result.source.is_char_boundary(pair[1])
                {
                    return Err(SearchError::InvalidHit);
                }
                captures.push(Some(pair[0]..pair[1]));
            }
        }
        let range = captures
            .first()
            .and_then(Clone::clone)
            .ok_or(SearchError::InvalidHit)?;
        let size = std::mem::size_of::<Hit>()
            + captures.len() * std::mem::size_of::<Option<Range<usize>>>();
        if result.hits.len() >= MAX_HITS || retained + size > MAX_RESULT_BYTES {
            result.state = SearchState::HitLimit;
            break;
        }
        retained += size;
        let next = if range.start == range.end {
            result.source[range.end..]
                .chars()
                .next()
                .map(|c| range.end + c.len_utf8())
        } else {
            Some(range.end)
        };
        result.hits.push(Hit { range, captures });
        match next {
            Some(next) => offset = next,
            None => break,
        }
    }
    Ok(result)
}
impl Results {
    pub fn is_current(&self, doc: &Document, generation: u64) -> bool {
        self.document_id == doc.id()
            && self.revision == doc.revision()
            && self.generation == generation
            && self.source.as_ref() == doc.source()
    }
    pub fn replace(
        &self,
        doc: &mut Document,
        generation: u64,
        index: Option<usize>,
        replacement: &str,
    ) -> Result<usize, SearchError> {
        if !self.is_current(doc, generation) {
            return Err(SearchError::Stale);
        }
        if self.state != SearchState::Complete {
            return Err(SearchError::Incomplete);
        }
        let hits = match index {
            Some(i) => std::slice::from_ref(self.hits.get(i).ok_or(SearchError::InvalidHit)?),
            None => &self.hits,
        };
        if hits.is_empty() {
            return Ok(0);
        }
        let start = hits[0].range.start;
        let end = hits.last().unwrap().range.end;
        let mut cursor = start;
        let mut output = String::new();
        for hit in hits {
            if hit.range.start < cursor {
                return Err(SearchError::InvalidHit);
            }
            append(&mut output, &self.source[cursor..hit.range.start])?;
            if self.query.regex {
                format_replacement(&self.source, hit, replacement, &mut output)?;
            } else {
                append(&mut output, replacement)?;
            }
            cursor = hit.range.end;
        }
        append(&mut output, &self.source[cursor..end])?;
        let caret = start + output.len();
        doc.replace_range_with_selection(start..end, &output, Selection::caret(caret))
            .map_err(|_| SearchError::InvalidHit)?;
        Ok(hits.len())
    }
}
fn append(output: &mut String, text: &str) -> Result<(), SearchError> {
    if output.len().saturating_add(text.len()) > MAX_PLAN_BYTES {
        return Err(SearchError::ReplacementTooLarge);
    }
    output.push_str(text);
    Ok(())
}
fn format_replacement(
    source: &str,
    hit: &Hit,
    replacement: &str,
    output: &mut String,
) -> Result<(), SearchError> {
    let mut chars = replacement.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '$' {
            append(output, c.encode_utf8(&mut [0; 4]))?;
            continue;
        }
        let value = match chars.peek().copied() {
            Some('$') => Some("$"),
            Some('&' | '0') => Some(&source[hit.range.clone()]),
            Some('`') => Some(&source[..hit.range.start]),
            Some('\'') => Some(&source[hit.range.end..]),
            Some(d @ '1'..='9') => Some(
                hit.captures
                    .get((d as u8 - b'0') as usize)
                    .and_then(Option::as_ref)
                    .map_or("", |r| &source[r.clone()]),
            ),
            _ => None,
        };
        if let Some(value) = value {
            chars.next();
            append(output, value)?;
        } else {
            append(output, "$")?;
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lookaround_backreference_unicode_captures_and_literal_replacement() {
        let mut d = Document::new(1, "日 😀 abc abc 日\r\n".into());
        let q = Query {
            text: "(?<=😀 )(abc) \\1(?= 日)".into(),
            regex: true,
            ..Default::default()
        };
        let results = search_document(&d, q, 1, &AtomicBool::new(false)).unwrap();
        assert_eq!(results.hits.len(), 1);
        assert_eq!(&d.source()[results.hits[0].range.clone()], "abc abc");
        results.replace(&mut d, 1, None, "$1/$$/$&").unwrap();
        assert_eq!(d.source(), "日 😀 abc/$/abc abc 日\r\n");
        assert_eq!(d.undo_count(), 1);
        d.undo();
        assert!(!d.is_dirty());
        assert!(results.replace(&mut d, 1, None, "x").is_err());
        let result = search_document(
            &d,
            Query {
                text: "日".into(),
                ..Default::default()
            },
            2,
            &AtomicBool::new(false),
        )
        .unwrap();
        result.replace(&mut d, 2, None, "$1").unwrap();
        assert_eq!(d.source(), "$1 😀 abc abc $1\r\n");
    }

    #[test]
    fn literal_query_over_regex_limit_matches_and_replaces_with_one_undo() {
        let needle = "x".repeat(5000);
        let original = format!("before {needle} after");
        let mut doc = Document::new(77, original.clone());
        let result = search_document(
            &doc,
            Query {
                text: needle.clone(),
                ..Default::default()
            },
            4,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(result.hits.len(), 1);
        assert_eq!(result.query.text, needle);
        assert_eq!(result.replace(&mut doc, 4, Some(0), "match").unwrap(), 1);
        assert_eq!(doc.source(), "before match after");
        assert_eq!(doc.undo_count(), 1);
        assert!(doc.undo());
        assert_eq!(doc.source(), original);

        let regex = search_document(
            &doc,
            Query {
                text: "x".repeat(5000),
                regex: true,
                ..Default::default()
            },
            5,
            &AtomicBool::new(false),
        );
        assert!(matches!(regex, Err(SearchError::PatternTooLong)));
    }

    #[test]
    fn unicode_case_insensitive_literal_still_escapes_regex_metacharacters() {
        let doc = Document::new(79, "A+B [x] 猫".into());
        let result = search_document(
            &doc,
            Query {
                text: "a+b [x] 猫".into(),
                case_sensitive: false,
                ..Default::default()
            },
            1,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(result.hits.len(), 1);
        assert_eq!(&doc.source()[result.hits[0].range.clone()], "A+B [x] 猫");
    }

    #[test]
    fn literal_query_has_a_finite_input_limit_and_reports_error() {
        let query = Query {
            text: "x".repeat(MAX_LITERAL_QUERY_BYTES + 1),
            ..Default::default()
        };
        let error = match search_document(
            &Document::new(78, String::new()),
            query,
            1,
            &AtomicBool::new(false),
        ) {
            Err(error) => error,
            Ok(_) => panic!("oversized literal query was accepted"),
        };
        assert!(matches!(error, SearchError::QueryTooLong));
        assert!(error.to_string().contains("8 MiB"));
    }
    #[test]
    fn boundaries_zero_length_cancel_deadline_and_pathological_regex_are_bounded() {
        let d = Document::new(1, "日 日本 日😀".into());
        let result = search_document(
            &d,
            Query {
                text: "日".into(),
                word: true,
                ..Default::default()
            },
            0,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(result.hits.len(), 2);
        let result = search_document(
            &d,
            Query {
                text: "(?=.)".into(),
                regex: true,
                ..Default::default()
            },
            0,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(result.hits.len(), d.source().chars().count());
        let cancelled = search_document(
            &d,
            Query {
                text: "日".into(),
                ..Default::default()
            },
            0,
            &AtomicBool::new(true),
        )
        .unwrap();
        assert_eq!(cancelled.state, SearchState::Cancelled);
        let expired = search_snapshot(
            1,
            0,
            d.source_shared(),
            Query {
                text: "日".into(),
                ..Default::default()
            },
            0,
            &AtomicBool::new(false),
            Duration::ZERO,
        )
        .unwrap();
        assert_eq!(expired.state, SearchState::TimedOut);
        let start = Instant::now();
        let pathological = search_snapshot(
            1,
            0,
            Arc::from(format!("{}!", "a".repeat(20000))),
            Query {
                text: "(a+)+$".into(),
                regex: true,
                ..Default::default()
            },
            0,
            &AtomicBool::new(false),
            Duration::from_millis(200),
        )
        .unwrap();
        assert!(matches!(
            pathological.state,
            SearchState::ResourceLimit | SearchState::TimedOut
        ));
        assert!(start.elapsed() < Duration::from_secs(2));
    }
}
