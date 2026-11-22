use astra_core::span::Span;

#[must_use]
pub fn offset_to_position(text: &str, offset: usize) -> (u32, u32) {
    let offset = offset.min(text.len());
    let mut offset = offset;
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }

    let mut line = 0u32;
    let mut character = 0u32;
    let mut index = 0usize;
    for ch in text.chars() {
        if index >= offset {
            break;
        }
        let len = ch.len_utf8();
        if index + len > offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            character = 0;
        } else if ch != '\r' {
            character += ch.len_utf16() as u32;
        }
        index += len;
    }
    (line, character)
}

#[must_use]
pub fn position_to_offset(text: &str, line: u32, character: u32) -> usize {
    let mut current_line = 0u32;
    let mut index = 0usize;
    while current_line < line && index < text.len() {
        let ch = text[index..].chars().next().expect("in-bounds char");
        index += ch.len_utf8();
        if ch == '\n' {
            current_line += 1;
        }
    }
    if current_line < line {
        return text.len();
    }

    let mut units = 0u32;
    while index < text.len() {
        let ch = text[index..].chars().next().expect("in-bounds char");
        if ch == '\n' || ch == '\r' {
            break;
        }
        let width = ch.len_utf16() as u32;
        if units + width > character {
            break;
        }
        units += width;
        index += ch.len_utf8();
    }
    index
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Word {
    pub text: String,
    pub start: usize,
    pub end: usize,
}

impl Word {
    #[must_use]
    pub fn span(&self) -> Span {
        Span::new(self.start, self.end)
    }
}

#[must_use]
pub fn is_identifier_char(ch: char) -> bool {
    ch == '_' || ch.is_alphanumeric()
}

#[must_use]
pub fn identifier_at(text: &str, offset: usize) -> Option<Word> {
    let offset = offset.min(text.len());
    let mut start = offset;
    while start > 0 {
        let previous = text[..start].chars().next_back()?;
        if !is_identifier_char(previous) {
            break;
        }
        start -= previous.len_utf8();
    }

    let mut end = offset;
    while end < text.len() {
        let next = text[end..].chars().next()?;
        if !is_identifier_char(next) {
            break;
        }
        end += next.len_utf8();
    }

    if start == end {
        None
    } else {
        Some(Word {
            text: text[start..end].to_owned(),
            start,
            end,
        })
    }
}

#[must_use]
pub fn identifier_before_dot(text: &str, offset: usize) -> Option<Word> {
    let mut before = text[..offset.min(text.len())].trim_end();
    if !before.ends_with('.') {
        return None;
    }
    before = before[..before.len() - 1].trim_end();
    identifier_at(before, before.len())
}

#[must_use]
pub fn name_span(text: &str, span: Span, name: &str) -> Span {
    let start = span.start.min(text.len());
    let end = span.end.min(text.len());
    if start >= end || name.is_empty() {
        return span;
    }

    let slice = &text[start..end];
    for (relative, _) in slice.match_indices(name) {
        let name_start = start + relative;
        let name_end = name_start + name.len();
        let before_ok = text[..name_start]
            .chars()
            .next_back()
            .is_none_or(|ch| !is_identifier_char(ch));
        let after_ok = text[name_end..]
            .chars()
            .next()
            .is_none_or(|ch| !is_identifier_char(ch));
        if before_ok && after_ok {
            return Span::new(name_start, name_end);
        }
    }
    span
}
