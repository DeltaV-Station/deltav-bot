use std::cell::LazyCell;

use regex::Regex;

pub const HTML_COMMENT_REGEX: LazyCell<Regex> =
    LazyCell::new(|| Regex::new("<!--([\\S\\s]*?)-->").unwrap());

pub fn remove_html_comments(value: impl AsRef<str>) -> String {
    HTML_COMMENT_REGEX
        .replace_all(value.as_ref(), "")
        .to_string()
}
