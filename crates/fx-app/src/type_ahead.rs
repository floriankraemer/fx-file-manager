//! Type-to-select: a typed-character buffer and the prefix match it drives.

use std::borrow::Cow;
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Debug, Default)]
pub(crate) struct TypeAhead {
    buffer: String,
    last: Option<Instant>,
}

impl TypeAhead {
    /// Appends `text` to the buffer, which restarts after a pause. Control characters and leading whitespace are dropped.
    pub(crate) fn push(&mut self, text: &str, now: Instant) -> &str {
        if self.last.is_none_or(|last| now.duration_since(last) >= TIMEOUT) {
            self.buffer.clear();
        }
        self.last = Some(now);
        let typed = text.chars().filter(|c| !c.is_control());
        for c in typed {
            if !(self.buffer.is_empty() && c.is_whitespace()) {
                self.buffer.push(c);
            }
        }
        &self.buffer
    }

    /// Feeds `text` into the buffer and returns the row of `len` rows it selects, see [`best_match`].
    pub(crate) fn find<'a>(
        &mut self,
        text: &str,
        now: Instant,
        current: Option<usize>,
        len: usize,
        name_at: impl Fn(usize) -> Option<Cow<'a, str>>,
    ) -> Option<usize> {
        let query = self.push(text, now);
        best_match(len, name_at, query, current)
    }
}

/// First row, starting at `current` (inclusive) and wrapping, whose name starts with `query` ignoring case.
/// A query of one repeated character (`aa`) cycles: the search starts after `current` and looks for that character.
pub(crate) fn best_match<'a>(
    len: usize,
    name_at: impl Fn(usize) -> Option<Cow<'a, str>>,
    query: &str,
    current: Option<usize>,
) -> Option<usize> {
    let query: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    let first = *query.first()?;
    let cycling = query.len() > 1 && query.iter().all(|&c| c == first);
    let (needle, start) = match (cycling, current) {
        (true, Some(row)) => (&query[..1], row + 1),
        (_, row) => (&query[..], row.unwrap_or(0)),
    };
    (0..len)
        .map(|i| (start + i) % len)
        .find(|&row| name_at(row).is_some_and(|name| starts_with_ci(&name, needle)))
}

fn starts_with_ci(name: &str, needle: &[char]) -> bool {
    let mut lowered = name.chars().flat_map(char::to_lowercase);
    needle.iter().all(|&c| lowered.next() == Some(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(names: &[&str], query: &str, current: Option<usize>) -> Option<usize> {
        best_match(
            names.len(),
            |i| names.get(i).map(|n| Cow::Borrowed(*n)),
            query,
            current,
        )
    }

    const NAMES: [&str; 5] = ["Alpha", "apple", "Beta", "Äpfel", "alps"];

    #[test]
    fn prefix_ignores_case() {
        assert_eq!(find(&NAMES, "BE", None), Some(2));
        assert_eq!(find(&NAMES, "alp", None), Some(0));
    }

    #[test]
    fn starts_at_current_inclusive_and_wraps() {
        assert_eq!(find(&NAMES, "al", Some(1)), Some(4));
        assert_eq!(find(&NAMES, "al", Some(4)), Some(4));
        assert_eq!(find(&NAMES, "be", Some(3)), Some(2));
    }

    #[test]
    fn repeated_character_cycles() {
        assert_eq!(find(&NAMES, "aa", Some(0)), Some(1));
        assert_eq!(find(&NAMES, "aaa", Some(1)), Some(4));
        assert_eq!(find(&NAMES, "aaaa", Some(4)), Some(0));
    }

    #[test]
    fn no_match_and_empty() {
        assert_eq!(find(&NAMES, "zz", Some(1)), None);
        assert_eq!(find(&NAMES, "", Some(1)), None);
        assert_eq!(find(&[], "a", None), None);
    }

    #[test]
    fn unicode_is_case_insensitive() {
        assert_eq!(find(&NAMES, "äp", None), Some(3));
        assert_eq!(find(&["ÉCOLE"], "éc", None), Some(0));
    }

    #[test]
    fn buffer_resets_after_timeout() {
        let mut t = TypeAhead::default();
        let t0 = Instant::now();
        assert_eq!(t.push("a", t0), "a");
        assert_eq!(t.push("b", t0 + Duration::from_millis(900)), "ab");
        assert_eq!(t.push("c", t0 + Duration::from_millis(2000)), "c");
    }

    #[test]
    fn leading_space_and_control_ignored() {
        let mut t = TypeAhead::default();
        assert_eq!(t.push(" \n", Instant::now()), "");
        assert_eq!(t.push("a b", Instant::now()), "a b");
    }
}
