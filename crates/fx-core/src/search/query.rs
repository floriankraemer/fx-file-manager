//! What matches: the query language (substring or glob), its case rules and match highlighting.

use std::ops::Range;

/// What a glob pattern is matched against. Substring queries always match the name.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GlobScope {
    #[default]
    Name,
    /// Path relative to the search root, with `/` as separator on every OS.
    RelativePath,
}

/// Case-insensitive (ASCII folding) by default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SearchOptions {
    pub scope: GlobScope,
    pub case_sensitive: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Pattern {
    Substring(Vec<u8>),
    Glob(Vec<u8>),
}

/// A substring, or a glob when the text contains `*` or `?`. An empty query matches everything.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchQuery {
    /// Lowercased unless the query is case sensitive.
    pattern: Pattern,
    options: SearchOptions,
}

impl SearchQuery {
    pub fn new(text: &str, options: SearchOptions) -> Self {
        let bytes = if options.case_sensitive {
            text.as_bytes().to_vec()
        } else {
            text.as_bytes().to_ascii_lowercase()
        };
        let pattern = if bytes.iter().any(|&b| b == b'*' || b == b'?') {
            Pattern::Glob(bytes)
        } else {
            Pattern::Substring(bytes)
        };
        Self { pattern, options }
    }

    pub fn is_glob(&self) -> bool {
        matches!(self.pattern, Pattern::Glob(_))
    }

    /// Byte range of the first substring match in `name`, for highlighting; always on character
    /// boundaries. `None` for globs, the empty query and names that do not match.
    pub fn match_range(&self, name: &str) -> Option<Range<usize>> {
        let Pattern::Substring(needle) = &self.pattern else {
            return None;
        };
        if needle.is_empty() {
            return None;
        }
        let start = find(name.as_bytes(), needle, self.options.case_sensitive)?;
        Some(start..start + needle.len())
    }

    pub(super) fn matches_name(&self, name: &[u8]) -> bool {
        match &self.pattern {
            Pattern::Substring(needle) => find(name, needle, self.options.case_sensitive).is_some(),
            Pattern::Glob(glob) => glob_match(glob, name, self.options.case_sensitive),
        }
    }

    pub(super) fn matches_relative(&self) -> bool {
        self.options.scope == GlobScope::RelativePath && self.is_glob()
    }
}

/// Position of the first occurrence of `needle`; `needle` is already lowercase unless `case_sensitive`.
/// Scans for the first byte before comparing the rest.
fn find(haystack: &[u8], needle: &[u8], case_sensitive: bool) -> Option<usize> {
    let Some((&first, rest)) = needle.split_first() else {
        return Some(0);
    };
    let last_start = haystack.len().checked_sub(needle.len())?;
    let (lower, upper) = match case_sensitive {
        true => (first, first),
        false => (first, first.to_ascii_uppercase()),
    };
    (0..=last_start).find(|&i| {
        let b = haystack[i];
        (b == lower || b == upper)
            && tail_matches(&haystack[i + 1..i + needle.len()], rest, case_sensitive)
    })
}

fn tail_matches(candidate: &[u8], rest: &[u8], case_sensitive: bool) -> bool {
    match case_sensitive {
        true => candidate == rest,
        false => candidate.eq_ignore_ascii_case(rest),
    }
}

/// Length of the well-formed UTF-8 character at the start of `rest`; 1 for stray or truncated
/// bytes, so a non-UTF-8 name (Latin-1) never has a wildcard swallow the ASCII that follows.
fn char_len(rest: &[u8]) -> usize {
    let len = match rest[0] {
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => return 1,
    };
    match rest.get(1..len) {
        Some(tail) if tail.iter().all(|b| (0x80..=0xBF).contains(b)) => len,
        _ => 1,
    }
}

/// Anchored glob match: `*` is any run, `?` one UTF-8 character. `glob` is already lowercase
/// unless `case_sensitive`. Iterative with single-star backtracking: O(glob * text) worst case,
/// no allocation.
fn glob_match(glob: &[u8], text: &[u8], case_sensitive: bool) -> bool {
    let fold = |b: u8| {
        if case_sensitive {
            b
        } else {
            b.to_ascii_lowercase()
        }
    };
    let (mut g, mut t) = (0, 0);
    let mut resume: Option<(usize, usize)> = None;
    while t < text.len() {
        match glob.get(g) {
            Some(b'*') => {
                resume = Some((g + 1, t));
                g += 1;
            }
            Some(b'?') => {
                g += 1;
                t = (t + char_len(&text[t..])).min(text.len());
            }
            Some(&c) if c == fold(text[t]) => {
                g += 1;
                t += 1;
            }
            _ => {
                let Some((rg, rt)) = resume else {
                    return false;
                };
                let next = (rt + char_len(&text[rt..])).min(text.len());
                resume = Some((rg, next));
                g = rg;
                t = next;
            }
        }
    }
    glob[g..].iter().all(|&c| c == b'*')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(text: &str) -> SearchQuery {
        SearchQuery::new(text, SearchOptions::default())
    }

    fn exact(text: &str) -> SearchQuery {
        SearchQuery::new(
            text,
            SearchOptions {
                case_sensitive: true,
                ..SearchOptions::default()
            },
        )
    }

    #[test]
    fn substring_ignores_ascii_case() {
        assert!(q("read").matches_name(b"README.md"));
        assert!(q("ME.m").matches_name(b"readme.md"));
        assert!(!q("readme").matches_name(b"read.md"));
    }

    #[test]
    fn case_sensitive_substring_and_glob() {
        assert!(exact("READ").matches_name(b"README.md"));
        assert!(!exact("read").matches_name(b"README.md"));
        assert!(exact("*.RS").matches_name(b"main.RS"));
        assert!(!exact("*.RS").matches_name(b"main.rs"));
        assert!(!exact("M?in").matches_name(b"main"));
    }

    #[test]
    fn empty_query_matches_everything() {
        assert!(q("").matches_name(b"anything"));
        assert!(q("").matches_name(b""));
    }

    #[test]
    fn needle_longer_than_name_does_not_match() {
        assert!(!q("readme").matches_name(b"read"));
        assert!(!q("a").matches_name(b""));
    }

    #[test]
    fn glob_star_and_question_mark() {
        assert!(q("*.rs").matches_name(b"main.RS"));
        assert!(!q("*.rs").matches_name(b"main.rs.bak"));
        assert!(q("a?c").matches_name(b"abc"));
        assert!(!q("a?c").matches_name(b"ac"));
        assert!(!q("a?c").matches_name(b"abbc"));
        assert!(q("a*b*c").matches_name(b"aXXbYYc"));
        assert!(q("*").matches_name(b""));
        assert!(!q("*.rs").matches_name(b"readme.md"));
    }

    #[test]
    fn question_mark_consumes_a_whole_utf8_character() {
        assert!(q("h?llo").matches_name("héllo".as_bytes()));
        assert!(q("*.txt").matches_name("日本語.txt".as_bytes()));
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_names_still_match() {
        let name = b"caf\xE9 notes.TXT";
        assert!(q("notes").matches_name(name));
        assert!(q("*.txt").matches_name(name));
        assert!(q("caf?*").matches_name(name));
        assert!(q("caf?e").matches_name(b"caf\xE9e"));
        assert!(!q("caf?").matches_name(b"caf\xE9e"));
        assert!(q("*?e").matches_name(b"caf\xE9e"));
    }

    #[test]
    fn relative_scope_applies_to_globs_only() {
        let relative = SearchOptions {
            scope: GlobScope::RelativePath,
            ..SearchOptions::default()
        };
        let glob = SearchQuery::new("src/*.rs", relative);
        assert!(glob.matches_relative());
        assert!(!SearchQuery::new("src", relative).matches_relative());
        assert!(glob.matches_name(b"src/main.rs"));
    }

    #[test]
    fn is_glob_follows_wildcards() {
        assert!(q("*.rs").is_glob());
        assert!(q("a?").is_glob());
        assert!(!q("plain").is_glob());
        assert!(!q("").is_glob());
    }

    #[test]
    fn match_range_is_the_first_substring_match() {
        assert_eq!(q("read").match_range("README read"), Some(0..4));
        assert_eq!(q("ME").match_range("readme"), Some(4..6));
        assert_eq!(exact("me").match_range("ME me"), Some(3..5));
        assert_eq!(q("zzz").match_range("readme"), None);
    }

    #[test]
    fn match_range_is_none_for_globs_and_the_empty_query() {
        assert_eq!(q("*.rs").match_range("main.rs"), None);
        assert_eq!(q("").match_range("main.rs"), None);
    }

    #[test]
    fn match_range_stays_on_character_boundaries() {
        let name = "日本語メモ.txt";
        let range = q("メモ").match_range(name).unwrap();
        assert_eq!(&name[range], "メモ");
        let range = q("É").match_range("café É").unwrap_or(0..0);
        assert!("café É".is_char_boundary(range.start) && "café É".is_char_boundary(range.end));
        let range = q("é").match_range("CAFé").unwrap();
        assert_eq!(&"CAFé"[range], "é");
    }
}
