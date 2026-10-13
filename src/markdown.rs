//! The Markdown dialect shared by display projection and table editing.

use std::ops::Range;

use linkify::{LinkFinder, LinkKind};
use pulldown_cmark::Options;

/// A bare GFM autolink span measured in UTF-8 bytes of a parser text event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GfmAutolink {
    pub range: Range<usize>,
    pub destination: String,
    /// GFM URL autolinks require a specific preceding delimiter. Email and
    /// protocol autolinks are recognized within any text node.
    pub needs_url_boundary: bool,
}

/// Explicit parser extensions adopted by MDLite. CommonMark syntax is enabled
/// by default; the flags here add tables, strike-through, task markers, and
/// YAML-style front matter. GFM bare autolinks are source-mapped separately.
pub fn parser_options() -> Options {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_YAML_STYLE_METADATA_BLOCKS);
    options
}

/// Find the bare URL, `www.`, email, and `mailto:`/`xmpp:` autolinks from the
/// GFM autolink extension. The general-purpose scanner supplies byte spans and
/// handles URL punctuation; the small checks below keep its wider RFC rules
/// from silently becoming part of MDLite's dialect.
pub(crate) fn gfm_autolinks(text: &str) -> Vec<GfmAutolink> {
    let mut finder = LinkFinder::new();
    finder.kinds(&[LinkKind::Url]);
    finder.url_must_have_scheme(false);

    let mut found = Vec::new();
    for link in finder.links(text) {
        let start = link.start();
        let mut end = link.end();
        let (destination, needs_url_boundary) = match link.kind() {
            LinkKind::Url => {
                end = gfm_url_candidate_end(text, start).unwrap_or(end);
                end = trim_gfm_url_end(text, start, end);
                let Some(candidate) = text.get(start..end) else {
                    continue;
                };
                let Some((destination, needs_url_boundary)) = gfm_url_destination(candidate) else {
                    continue;
                };
                (destination, needs_url_boundary)
            }
            _ => continue,
        };
        if start < end {
            found.push(GfmAutolink {
                range: start..end,
                destination,
                needs_url_boundary,
            });
        }
    }
    for email in gfm_email_autolinks(text) {
        if !found.iter().any(|existing| {
            existing.range.start < email.range.end && email.range.start < existing.range.end
        }) {
            found.push(email);
        }
    }
    found.sort_by_key(|link| link.range.start);
    found
}

fn gfm_url_destination(candidate: &str) -> Option<(String, bool)> {
    for scheme in ["https://", "http://"] {
        if strip_ascii_prefix(candidate, scheme).is_some_and(valid_gfm_domain_prefix) {
            return Some((candidate.to_owned(), true));
        }
    }
    if let Some(rest) = strip_ascii_prefix(candidate, "www.")
        && valid_gfm_domain_prefix(rest)
    {
        return Some((format!("http://{candidate}"), true));
    }
    None
}

fn gfm_url_candidate_end(text: &str, start: usize) -> Option<usize> {
    let rest = text.get(start..)?;
    Some(
        rest.char_indices()
            .find(|(_, character)| character.is_whitespace() || *character == '<')
            .map_or(text.len(), |(offset, _)| start + offset),
    )
}

fn strip_ascii_prefix<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    value
        .get(..prefix.len())
        .filter(|start| start.eq_ignore_ascii_case(prefix))
        .and_then(|_| value.get(prefix.len()..))
}

fn protocol_before_email(text: &str, email_start: usize) -> Option<(usize, &'static str)> {
    ["mailto:", "xmpp:"].into_iter().find_map(|protocol| {
        let start = email_start.checked_sub(protocol.len())?;
        text.get(start..email_start)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(protocol))
            .then_some((start, protocol))
    })
}

fn xmpp_resource_end(text: &str, email_end: usize) -> Option<usize> {
    let resource = text.get(email_end..)?.strip_prefix('/')?;
    let resource_start = email_end + 1;
    let resource_len = resource
        .char_indices()
        .take_while(|(_, character)| {
            character.is_ascii_alphanumeric() || matches!(character, '@' | '.')
        })
        .map(|(offset, character)| offset + character.len_utf8())
        .last()?;
    (resource_len > 0).then_some(resource_start + resource_len)
}

fn gfm_email_autolinks(text: &str) -> Vec<GfmAutolink> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    for (at, _) in text
        .char_indices()
        .filter(|(_, character)| *character == '@')
    {
        let mut email_start = at;
        while email_start > 0 && is_gfm_email_local_byte(bytes[email_start - 1]) {
            email_start -= 1;
        }
        if email_start == at {
            continue;
        }

        let mut email_end = at + 1;
        while email_end < bytes.len() && is_gfm_email_domain_byte(bytes[email_end]) {
            email_end += 1;
        }
        if text
            .get(at + 1..email_end)
            .is_some_and(|domain| domain.ends_with('.'))
        {
            email_end -= 1;
        }
        let Some(email) = text.get(email_start..email_end) else {
            continue;
        };
        if !valid_gfm_email(email) {
            continue;
        }

        let (range_start, range_end, destination) =
            if let Some((protocol_start, protocol)) = protocol_before_email(text, email_start) {
                let protocol_end = if protocol.eq_ignore_ascii_case("xmpp:") {
                    xmpp_resource_end(text, email_end).unwrap_or(email_end)
                } else {
                    email_end
                };
                let Some(destination) = text.get(protocol_start..protocol_end) else {
                    continue;
                };
                (protocol_start, protocol_end, destination.to_owned())
            } else {
                (email_start, email_end, format!("mailto:{email}"))
            };
        found.push(GfmAutolink {
            range: range_start..range_end,
            destination,
            needs_url_boundary: false,
        });
    }
    found
}

fn is_gfm_email_local_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'+')
}

fn is_gfm_email_domain_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_')
}

fn valid_gfm_domain_prefix(value: &str) -> bool {
    let end = value
        .find(|character: char| {
            !(character.is_alphanumeric() || matches!(character, '.' | '_' | '-'))
        })
        .unwrap_or(value.len());
    value.get(..end).is_some_and(valid_gfm_domain)
}

fn valid_gfm_domain(domain: &str) -> bool {
    let labels = domain.split('.').collect::<Vec<_>>();
    if labels.len() < 2
        || labels.iter().any(|label| {
            label.is_empty()
                || !label
                    .chars()
                    .all(|character| character.is_alphanumeric() || matches!(character, '_' | '-'))
        })
    {
        return false;
    }

    !labels.iter().rev().take(2).any(|label| label.contains('_'))
}

fn valid_gfm_email(email: &str) -> bool {
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    if local.is_empty()
        || domain.is_empty()
        || domain.contains('@')
        || !local.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_' | '+')
        })
    {
        return false;
    }

    let labels = domain.split('.').collect::<Vec<_>>();
    labels.len() >= 2
        && labels.iter().all(|label| {
            !label.is_empty()
                && label.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
                })
        })
        && labels
            .last()
            .and_then(|label| label.chars().last())
            .is_some_and(|character| character.is_ascii_alphanumeric())
}

fn trim_gfm_url_end(text: &str, start: usize, mut end: usize) -> usize {
    while let Some(character) = text
        .get(start..end)
        .and_then(|value| value.chars().next_back())
    {
        if matches!(character, '?' | '!' | '.' | ',' | ':' | '*' | '_' | '~') {
            end -= character.len_utf8();
        } else {
            break;
        }
    }

    if text
        .get(start..end)
        .is_some_and(|value| value.ends_with(';'))
    {
        let candidate = text.get(start..end).unwrap_or_default();
        if let Some(ampersand) = candidate.rfind('&') {
            let entity_name = candidate.get(ampersand + 1..candidate.len() - 1);
            if entity_name.is_some_and(|name| {
                !name.is_empty()
                    && name
                        .chars()
                        .all(|character| character.is_ascii_alphanumeric())
            }) {
                end = start + ampersand;
            }
        }
    }

    loop {
        if text
            .get(start..end)
            .and_then(|value| value.chars().next_back())
            != Some(')')
        {
            break;
        }
        let candidate = text.get(start..end).unwrap_or_default();
        let opening = candidate
            .chars()
            .filter(|character| *character == '(')
            .count();
        let closing = candidate
            .chars()
            .filter(|character| *character == ')')
            .count();
        if closing <= opening {
            break;
        }
        end -= 1;
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_flags_are_a_small_explicit_subset() {
        let options = parser_options();
        assert!(Options::ENABLE_OLD_FOOTNOTES.contains(Options::ENABLE_FOOTNOTES));
        let mut defined_names = Options::from_bits_retain(u32::MAX)
            .iter_names()
            .map(|(name, _)| name)
            .collect::<Vec<_>>();
        defined_names.sort_unstable();
        assert_eq!(
            defined_names,
            [
                "ENABLE_DEFINITION_LIST",
                "ENABLE_FOOTNOTES",
                "ENABLE_GFM",
                "ENABLE_HEADING_ATTRIBUTES",
                "ENABLE_MATH",
                "ENABLE_OLD_FOOTNOTES",
                "ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS",
                "ENABLE_SMART_PUNCTUATION",
                "ENABLE_STRIKETHROUGH",
                "ENABLE_SUBSCRIPT",
                "ENABLE_SUPERSCRIPT",
                "ENABLE_TABLES",
                "ENABLE_TASKLISTS",
                "ENABLE_WIKILINKS",
                "ENABLE_YAML_STYLE_METADATA_BLOCKS",
            ]
        );

        let adopted = [
            Options::ENABLE_TABLES,
            Options::ENABLE_STRIKETHROUGH,
            Options::ENABLE_TASKLISTS,
            Options::ENABLE_YAML_STYLE_METADATA_BLOCKS,
        ];
        for option in adopted {
            assert!(options.contains(option));
        }

        let deferred = [
            Options::ENABLE_FOOTNOTES,
            Options::ENABLE_SMART_PUNCTUATION,
            Options::ENABLE_HEADING_ATTRIBUTES,
            Options::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS,
            Options::ENABLE_OLD_FOOTNOTES,
            Options::ENABLE_MATH,
            Options::ENABLE_GFM,
            Options::ENABLE_DEFINITION_LIST,
            Options::ENABLE_SUPERSCRIPT,
            Options::ENABLE_SUBSCRIPT,
            Options::ENABLE_WIKILINKS,
        ];
        for option in deferred {
            assert!(!options.intersects(option));
        }
    }

    #[test]
    fn extended_autolinks_follow_gfm_targets_boundaries_and_punctuation() {
        let text = concat!(
            "Visit https://example.org/a_(b)). and www.example.org/path. ",
            "Mail user+tag@example.org. Bad user@localhost and user@mail+tag.example. ",
            "unknown example.org and ftp://example.org."
        );
        let links = gfm_autolinks(text);
        let labels = links
            .iter()
            .map(|link| (&text[link.range.clone()], link.destination.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            labels,
            [
                ("https://example.org/a_(b)", "https://example.org/a_(b)"),
                ("www.example.org/path", "http://www.example.org/path"),
                ("user+tag@example.org", "mailto:user+tag@example.org"),
            ]
        );
        assert!(links[..2].iter().all(|link| link.needs_url_boundary));
        assert!(!links[2].needs_url_boundary);

        let boundary = "prefixhttps://example.org and (https://example.org";
        let links = gfm_autolinks(boundary);
        assert_eq!(links.len(), 1);
        assert_eq!(&boundary[links[0].range.clone()], "https://example.org");

        let entity_suffix = "www.example.org/search?q=commonmark&hl;";
        let links = gfm_autolinks(entity_suffix);
        assert_eq!(links.len(), 1);
        assert_eq!(
            &entity_suffix[links[0].range.clone()],
            "www.example.org/search?q=commonmark"
        );

        let iri_url = "https://例え.jp/道";
        let links = gfm_autolinks(iri_url);
        assert_eq!(links.len(), 1);
        assert_eq!(&iri_url[links[0].range.clone()], iri_url);
        assert_eq!(links[0].destination, iri_url);
    }

    #[test]
    fn extended_email_and_protocol_syntax_obeys_gfm_character_rules() {
        let text = concat!(
            "foo@bar_.example is valid, foo@bar.example_ is not; ",
            "mailto:foo@bar.example, xmpp:foo@bar.example/txt@bin"
        );
        let links = gfm_autolinks(text);
        let labels = links
            .iter()
            .map(|link| (&text[link.range.clone()], link.destination.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            labels,
            [
                ("foo@bar_.example", "mailto:foo@bar_.example"),
                ("mailto:foo@bar.example", "mailto:foo@bar.example"),
                (
                    "xmpp:foo@bar.example/txt@bin",
                    "xmpp:foo@bar.example/txt@bin"
                ),
            ]
        );
        assert!(links.iter().all(|link| !link.needs_url_boundary));

        let unicode_email = "猫@example.org and user@例え.jp";
        assert!(gfm_autolinks(unicode_email).is_empty());
    }
}
