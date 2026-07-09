use super::*;

fn chars(s: &str) -> Vec<char> {
    s.chars().collect()
}

#[test]
fn bare_url() {
    let cells = chars("visit https://example.com now");
    let urls = find_urls(&cells);
    assert_eq!(urls.len(), 1);
    let (start, end) = urls[0];
    let text: String = cells[start..end].iter().collect();
    assert_eq!(text, "https://example.com");
}

#[test]
fn trims_trailing_punctuation() {
    let cells = chars("see https://example.com.");
    let urls = find_urls(&cells);
    assert_eq!(urls.len(), 1);
    let (start, end) = urls[0];
    let text: String = cells[start..end].iter().collect();
    assert_eq!(text, "https://example.com");
}

#[test]
fn trims_unmatched_wrapping_paren() {
    let cells = chars("(https://example.com)");
    let urls = find_urls(&cells);
    assert_eq!(urls.len(), 1);
    let (start, end) = urls[0];
    let text: String = cells[start..end].iter().collect();
    assert_eq!(text, "https://example.com");
}

#[test]
fn keeps_parens_that_are_part_of_the_url() {
    let cells = chars("https://example.com/wiki/Rust_(programming_language)");
    let urls = find_urls(&cells);
    assert_eq!(urls.len(), 1);
    let (start, end) = urls[0];
    let text: String = cells[start..end].iter().collect();
    assert_eq!(text, "https://example.com/wiki/Rust_(programming_language)");
}

#[test]
fn no_match() {
    let cells = chars("no links in this line at all");
    assert!(find_urls(&cells).is_empty());
}

#[test]
fn keeps_query_string() {
    let cells = chars("go to http://example.com/foo?a=1&b=2 please");
    let urls = find_urls(&cells);
    assert_eq!(urls.len(), 1);
    let (start, end) = urls[0];
    let text: String = cells[start..end].iter().collect();
    assert_eq!(text, "http://example.com/foo?a=1&b=2");
}

#[test]
fn link_at_hits_the_right_column() {
    let cells = chars("see https://example.com.");
    // "https://example.com" starts at column 4.
    assert_eq!(link_at(&cells, 4), Some("https://example.com".to_string()));
    assert_eq!(link_at(&cells, 10), Some("https://example.com".to_string()));
    assert_eq!(link_at(&cells, 0), None);
}

#[test]
fn select_at_picks_the_whole_url_over_the_word_under_the_click() {
    let cells = chars("see https://example.com.");
    // A double-click anywhere in the URL selects the whole thing, not just the word
    // `example` (word chars alone would stop at the dots and slashes).
    let (start, end) = select_at(&cells, 14).unwrap();
    let text: String = cells[start..end].iter().collect();
    assert_eq!(text, "https://example.com");
}

#[test]
fn select_at_falls_back_to_a_generic_word_outside_any_url() {
    let cells = chars("hello world");
    let (start, end) = select_at(&cells, 8).unwrap();
    let text: String = cells[start..end].iter().collect();
    assert_eq!(text, "world");
}

#[test]
fn select_at_is_none_on_whitespace() {
    let cells = chars("hello world");
    assert_eq!(select_at(&cells, 5), None);
}

#[test]
fn quote_ends_the_url_even_with_no_space_after_it() {
    // A quoted URL glued directly to trailing text (no space) must not swallow the
    // closing quote or the text after it.
    let cells = chars("\"http://www.google.com\"asd");
    let urls = find_urls(&cells);
    assert_eq!(urls.len(), 1);
    let (start, end) = urls[0];
    let text: String = cells[start..end].iter().collect();
    assert_eq!(text, "http://www.google.com");
}

#[test]
fn single_quote_also_ends_the_url() {
    let cells = chars("'http://www.google.com'asd");
    let urls = find_urls(&cells);
    assert_eq!(urls.len(), 1);
    let (start, end) = urls[0];
    let text: String = cells[start..end].iter().collect();
    assert_eq!(text, "http://www.google.com");
}
