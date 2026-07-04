use lightningcss::rules::Location;

use crate::source::{SourceId, Span};

pub(super) struct LineIndex<'source> {
    source: &'source str,
    source_id: SourceId,
    offset: usize,
    line_starts: Vec<usize>,
}

impl<'source> LineIndex<'source> {
    pub(super) fn with_offset(source: &'source str, source_id: SourceId, offset: usize) -> Self {
        let mut line_starts = vec![0];
        for (index, byte) in source.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(index + 1);
            }
        }

        Self {
            source,
            source_id,
            offset,
            line_starts,
        }
    }

    pub(super) fn location_span(&self, line: u32, column: u32) -> Option<Span> {
        let start = self.byte_index(line, column)?;
        Some(Span::new(
            self.source_id,
            self.offset + start,
            self.offset + next_char_boundary(self.source, start),
        ))
    }

    pub(super) fn rule_span(&self, location: Location) -> Option<Span> {
        let start = self.byte_index(location.line, location.column)?;
        let end = find_rule_end(self.source, start).unwrap_or(self.source.len());
        Some(Span::new(
            self.source_id,
            self.offset + start,
            self.offset + end,
        ))
    }

    fn byte_index(&self, line: u32, column: u32) -> Option<usize> {
        let line = usize::try_from(line).ok()?;
        let start = *self.line_starts.get(line)?;
        let end = self
            .line_starts
            .get(line + 1)
            .copied()
            .map(|next| next.saturating_sub(1))
            .unwrap_or_else(|| self.source.len());
        let target_column = column.saturating_sub(1);
        let mut current_column = 0;

        for (offset, ch) in self.source[start..end].char_indices() {
            if current_column >= target_column {
                return Some(start + offset);
            }

            current_column += ch.len_utf16() as u32;
            if current_column > target_column {
                return Some(start + offset);
            }
        }

        Some(end)
    }
}

fn next_char_boundary(source: &str, start: usize) -> usize {
    source[start..]
        .chars()
        .next()
        .map_or(start, |ch| start + ch.len_utf8())
}

fn find_rule_end(source: &str, start: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut index = start;
    let mut depth = 0usize;
    let mut quote = None;
    let mut escaped = false;
    let mut in_comment = false;
    let mut found_block = false;

    while index < bytes.len() {
        let byte = bytes[index];

        if in_comment {
            if byte == b'*' && bytes.get(index + 1) == Some(&b'/') {
                in_comment = false;
                index += 2;
                continue;
            }
            index += 1;
            continue;
        }

        if let Some(quote_byte) = quote {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == quote_byte {
                quote = None;
            }
            index += 1;
            continue;
        }

        match byte {
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                in_comment = true;
                index += 2;
            }
            b'\'' | b'"' => {
                quote = Some(byte);
                index += 1;
            }
            b'{' => {
                found_block = true;
                depth += 1;
                index += 1;
            }
            b'}' if found_block => {
                depth = depth.saturating_sub(1);
                index += 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => index += 1,
        }
    }

    None
}
