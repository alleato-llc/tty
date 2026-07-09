//! URL detection over a terminal row's cells, and opening one in the OS default
//! handler. Kept regex-free: terminal rows are short, so a hand-rolled scan is both
//! simpler and cheaper than pulling in the `regex` crate for two literal schemes.
//!
//! Operates in char-space (one `char` per grid cell) so match ranges line up directly
//! with columns — no byte/char index translation needed at the call site.

const SCHEMES: [&str; 2] = ["https://", "http://"];
/// Trimmed off the end of a match — punctuation a URL is unlikely to end with when
/// it's really just trailing prose (a sentence's period, a comma in a list, etc).
const TRAILING_PUNCTUATION: [char; 6] = ['.', ',', ';', ':', '!', '?'];
/// A quote character immediately ends the scan (not trimmed off the end — it's a hard
/// stop): a quoted URL like `"http://x.com"asd` must not swallow the closing quote or
/// whatever follows it with no space, since that's outside the quotes entirely.
const QUOTE_STOPS: [char; 2] = ['"', '\''];

/// Every `http(s)://` run in `cells`, as `(start, end)` column ranges (`end`
/// exclusive). Runs extend to the next whitespace/control/quote char, then trim
/// trailing punctuation and one unmatched closing bracket (so `(https://x.com)` and
/// `see https://x.com.` don't pull in the wrapper).
pub fn find_urls(cells: &[char]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let n = cells.len();
    let mut i = 0;
    while i < n {
        let Some(scheme) = SCHEMES.iter().find(|s| starts_with(cells, i, s)) else {
            i += 1;
            continue;
        };
        let start = i;
        let mut end = i;
        while end < n
            && !cells[end].is_whitespace()
            && !cells[end].is_control()
            && !QUOTE_STOPS.contains(&cells[end])
        {
            end += 1;
        }
        while end > start && TRAILING_PUNCTUATION.contains(&cells[end - 1]) {
            end -= 1;
        }
        if let Some(bracket) = end
            .checked_sub(1)
            .and_then(|last| closing_pair(cells[last]))
        {
            let (open, close) = bracket;
            let opens = cells[start..end].iter().filter(|&&c| c == open).count();
            let closes = cells[start..end].iter().filter(|&&c| c == close).count();
            if closes > opens {
                end -= 1;
            }
        }
        if end - start > scheme.len() {
            out.push((start, end));
            i = end.max(start + 1);
        } else {
            i = start + 1;
        }
    }
    out
}

/// The URL under column `col`, if any.
pub fn link_at(cells: &[char], col: usize) -> Option<String> {
    link_span_at(cells, col).map(|(start, end)| cells[start..end].iter().collect())
}

/// The `(start, end)` column range of the URL under `col`, if any — the span version
/// of [`link_at`], for callers (hover, double-click) that need the range rather than
/// the text.
pub fn link_span_at(cells: &[char], col: usize) -> Option<(usize, usize)> {
    find_urls(cells)
        .into_iter()
        .find(|(start, end)| (*start..*end).contains(&col))
}

fn starts_with(cells: &[char], at: usize, needle: &str) -> bool {
    let needle: Vec<char> = needle.chars().collect();
    at + needle.len() <= cells.len() && cells[at..at + needle.len()] == needle[..]
}

fn closing_pair(c: char) -> Option<(char, char)> {
    match c {
        ')' => Some(('(', ')')),
        ']' => Some(('[', ']')),
        '}' => Some(('{', '}')),
        '>' => Some(('<', '>')),
        _ => None,
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// What a double-click at `col` should select, as an `(start, end)` column range
/// (`end` exclusive): the whole URL when `col` lands inside one (see [`link_at`]),
/// else the contiguous run of word characters around it. `None` on whitespace or
/// punctuation outside any URL — a double-click there selects nothing.
pub fn select_at(cells: &[char], col: usize) -> Option<(usize, usize)> {
    if let Some(span) = link_span_at(cells, col) {
        return Some(span);
    }
    if col >= cells.len() || !is_word_char(cells[col]) {
        return None;
    }
    let mut start = col;
    while start > 0 && is_word_char(cells[start - 1]) {
        start -= 1;
    }
    let mut end = col + 1;
    while end < cells.len() && is_word_char(cells[end]) {
        end += 1;
    }
    Some((start, end))
}

/// Open `url` in the OS default handler (browser, for `http(s)://`).
pub fn open(url: &str) -> std::io::Result<()> {
    open::that(url)
}

#[cfg(test)]
#[path = "link_tests.rs"]
mod tests;
