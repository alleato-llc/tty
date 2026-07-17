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

#[test]
fn file_link_matches_path_line_col() {
    let cells = chars("error at src/main.rs:42:10 here");
    let links = find_file_links(&cells);
    assert_eq!(links.len(), 1);
    let (start, end, link) = &links[0];
    assert_eq!(
        cells[*start..*end].iter().collect::<String>(),
        "src/main.rs:42:10"
    );
    assert_eq!(link.path, "src/main.rs");
    assert_eq!(link.line, Some(42));
    assert_eq!(link.col, Some(10));
}

#[test]
fn file_link_line_only_and_absolute() {
    // Line without a column.
    let l = file_link_at(&chars("./build.rs:7 warning"), 3).unwrap();
    assert_eq!(l.path, "./build.rs");
    assert_eq!((l.line, l.col), (Some(7), None));
    // Absolute path.
    let l = file_link_at(&chars("/Users/me/x.py:99:2"), 0).unwrap();
    assert_eq!(l.path, "/Users/me/x.py");
    assert_eq!((l.line, l.col), (Some(99), Some(2)));
}

#[test]
fn file_link_rejects_non_paths_and_urls() {
    // Bare word:number (no `/` or `.`) — e.g. a timestamp — is not a file link.
    assert!(find_file_links(&chars("took 12:34 minutes")).is_empty());
    assert!(find_file_links(&chars("foo:42")).is_empty());
    // A URL with a port is left to the URL matcher, not linkified as a file.
    assert!(find_file_links(&chars("https://x.com:8080/a")).is_empty());
    // A path with no line number isn't a link (too many false positives).
    assert!(find_file_links(&chars("edit src/main.rs now")).is_empty());
}

#[test]
fn file_link_span_covers_the_reference() {
    let cells = chars("  src/lib.rs:3:5");
    let (s, e) = file_link_span_at(&cells, 5).unwrap();
    assert_eq!(cells[s..e].iter().collect::<String>(), "src/lib.rs:3:5");
    // Outside the span → nothing.
    assert!(file_link_span_at(&cells, 0).is_none());
}
