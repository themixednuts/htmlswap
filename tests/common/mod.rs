pub fn emitted_child_signatures_in_stack_order(code: &str) -> Vec<String> {
    let mut signatures = Vec::new();
    let mut index = 0;

    while let Some(relative) = code[index..].find(".child(") {
        let call_start = index + relative;
        let open = call_start + ".child".len();
        let close = matching_paren(code, open).expect("child call should have matching parens");
        if let Some(signature) = child_signature(&code[open + 1..close]) {
            signatures.push(signature);
        }
        index = close + 1;
    }

    signatures
}

fn child_signature(source: &str) -> Option<String> {
    first_id_literal(source)
        .map(|id| format!("id:{id}"))
        .or_else(|| first_constructor_literal(source).map(|id| format!("new:{id}")))
        .or_else(|| first_child_text_literal(source).map(|text| format!("text:{text}")))
}

fn first_id_literal(source: &str) -> Option<String> {
    let mut index = 0;
    while let Some(relative) = source[index..].find(".id(") {
        let call_start = index + relative;
        let open = call_start + ".id".len();
        let close = matching_paren(source, open)?;
        if let Some(value) = rust_string_literal_value(&source[open + 1..close]) {
            return Some(value);
        }
        index = close + 1;
    }

    None
}

fn first_constructor_literal(source: &str) -> Option<String> {
    let mut index = 0;
    while let Some(relative) = source[index..].find("::new(") {
        let call_start = index + relative;
        let open = call_start + "::new".len();
        let close = matching_paren(source, open)?;
        if let Some(value) = rust_string_literal_value(first_argument(&source[open + 1..close])) {
            return Some(value);
        }
        index = close + 1;
    }

    None
}

fn first_child_text_literal(source: &str) -> Option<String> {
    if let Some(value) = rust_string_literal_value(source) {
        return Some(value);
    }

    let mut index = 0;
    while let Some(relative) = source[index..].find(".child(") {
        let call_start = index + relative;
        let open = call_start + ".child".len();
        let close = matching_paren(source, open)?;
        if let Some(value) = rust_string_literal_value(&source[open + 1..close]) {
            return Some(value);
        }
        index = close + 1;
    }

    None
}

fn first_argument(source: &str) -> &str {
    source.split_once(',').map_or(source, |(first, _)| first)
}

fn matching_paren(source: &str, open: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    if bytes.get(open) != Some(&b'(') {
        return None;
    }

    let mut stack = vec![b')'];
    let mut index = open + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => index = skip_string(bytes, index)?,
            b'/' if bytes.get(index + 1) == Some(&b'/') => index = skip_line_comment(bytes, index),
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index = skip_block_comment(bytes, index)?
            }
            b'(' => {
                stack.push(b')');
                index += 1;
            }
            b'[' => {
                stack.push(b']');
                index += 1;
            }
            b'{' => {
                stack.push(b'}');
                index += 1;
            }
            ch if stack.last().copied() == Some(ch) => {
                stack.pop();
                if stack.is_empty() {
                    return Some(index);
                }
                index += 1;
            }
            _ => index += 1,
        }
    }

    None
}

fn skip_string(bytes: &[u8], quote: usize) -> Option<usize> {
    let mut index = quote + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'"' => return Some(index + 1),
            _ => index += 1,
        }
    }

    None
}

fn skip_line_comment(bytes: &[u8], start: usize) -> usize {
    let mut index = start + 2;
    while index < bytes.len() && bytes[index] != b'\n' {
        index += 1;
    }
    index
}

fn skip_block_comment(bytes: &[u8], start: usize) -> Option<usize> {
    let mut index = start + 2;
    while index + 1 < bytes.len() {
        if bytes[index] == b'*' && bytes[index + 1] == b'/' {
            return Some(index + 2);
        }
        index += 1;
    }

    None
}

fn rust_string_literal_value(expression: &str) -> Option<String> {
    let expression = expression.trim();
    let expression = expression.strip_suffix(',').unwrap_or(expression).trim();
    let bytes = expression.as_bytes();
    if bytes.first() != Some(&b'"') {
        return None;
    }

    let end = skip_string(bytes, 0)?;
    if end != bytes.len() {
        return None;
    }

    let mut output = String::new();
    let mut escaped = false;
    for ch in expression[1..expression.len() - 1].chars() {
        if escaped {
            match ch {
                'n' => output.push('\n'),
                'r' => output.push('\r'),
                't' => output.push('\t'),
                '"' | '\\' => output.push(ch),
                _ => output.push(ch),
            }
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else {
            output.push(ch);
        }
    }

    (!escaped).then_some(output)
}
