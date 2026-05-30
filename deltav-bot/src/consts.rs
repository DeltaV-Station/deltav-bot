use std::cell::LazyCell;

use regex::Regex;

pub const HTML_COMMENT_REGEX: LazyCell<Regex> =
    LazyCell::new(|| Regex::new("<!--([\\S\\s]*?)-->").unwrap());
