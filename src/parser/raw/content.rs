//! PDF content stream parser.

use crate::error::Result;
use crate::parser::backend::{ContentOp, PdfValue};

/// Parse a content stream into a sequence of operations.
pub fn parse_content_stream(data: &[u8]) -> Result<Vec<ContentOp>> {
    let mut ops = Vec::new();
    let mut operand_stack: Vec<PdfValue> = Vec::new();
    let len = data.len();
    let mut i = 0;

    while i < len {
        // Skip whitespace
        if is_whitespace(data[i]) {
            i += 1;
            continue;
        }

        // Skip PDF comments (% until end of line)
        if data[i] == b'%' {
            while i < len && data[i] != b'\n' && data[i] != b'\r' {
                i += 1;
            }
            continue;
        }

        // Literal string (...)
        if data[i] == b'(' {
            let (val, next) = parse_literal_string(data, i);
            operand_stack.push(val);
            i = next;
            continue;
        }

        // Hex string <...>
        if data[i] == b'<' && i + 1 < len && data[i + 1] != b'<' {
            let (val, next) = parse_hex_string(data, i);
            operand_stack.push(val);
            i = next;
            continue;
        }

        // Array [...]
        if data[i] == b'[' {
            let (val, next) = parse_array(data, i);
            operand_stack.push(val);
            i = next;
            continue;
        }

        // Name /Foo
        if data[i] == b'/' {
            let (val, next) = parse_name(data, i);
            operand_stack.push(val);
            i = next;
            continue;
        }

        // Number (integer or real), or negative sign
        if data[i] == b'-' || data[i] == b'+' || data[i] == b'.' || data[i].is_ascii_digit() {
            let (val, next) = parse_number(data, i);
            operand_stack.push(val);
            i = next;
            continue;
        }

        // Alphabetic token → could be operator or keyword (true/false/null)
        if is_operator_start(data[i]) {
            let start = i;
            // Collect alphabetic characters
            while i < len && data[i].is_ascii_alphabetic() {
                i += 1;
            }
            // Check for * suffix (T*, b*, B*)
            if i < len && data[i] == b'*' {
                i += 1;
            }

            let token = &data[start..i];
            let token_str = std::str::from_utf8(token).unwrap_or("");

            match token_str {
                "true" | "false" | "null" => {
                    operand_stack.push(PdfValue::Other);
                }
                "BI" => {
                    // Inline image: skip until EI
                    i = skip_inline_image(data, i);
                    // Emit BI as an operator with no operands (stack should be empty)
                    ops.push(ContentOp {
                        operator: "BI".to_string(),
                        operands: std::mem::take(&mut operand_stack),
                    });
                }
                _ => {
                    // It's an operator
                    ops.push(ContentOp {
                        operator: token_str.to_string(),
                        operands: std::mem::take(&mut operand_stack),
                    });
                }
            }
            continue;
        }

        // Special single-char operators: ' and "
        if data[i] == b'\'' || data[i] == b'"' {
            let op = (data[i] as char).to_string();
            i += 1;
            ops.push(ContentOp {
                operator: op,
                operands: std::mem::take(&mut operand_stack),
            });
            continue;
        }

        // Skip unknown bytes
        i += 1;
    }

    Ok(ops)
}

fn is_whitespace(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0 | 12)
}

fn is_operator_start(b: u8) -> bool {
    b.is_ascii_alphabetic()
}

fn is_delimiter(b: u8) -> bool {
    matches!(
        b,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    ) || is_whitespace(b)
}

/// Parse a literal string starting at `(`, returns (PdfValue, next_index).
fn parse_literal_string(data: &[u8], start: usize) -> (PdfValue, usize) {
    let mut i = start + 1; // skip opening '('
    let mut result = Vec::new();
    let mut depth = 1;
    let len = data.len();

    while i < len && depth > 0 {
        match data[i] {
            b'(' => {
                depth += 1;
                result.push(b'(');
                i += 1;
            }
            b')' => {
                depth -= 1;
                if depth > 0 {
                    result.push(b')');
                }
                i += 1;
            }
            b'\\' if i + 1 < len => {
                i += 1;
                match data[i] {
                    b'n' => {
                        result.push(b'\n');
                        i += 1;
                    }
                    b'r' => {
                        result.push(b'\r');
                        i += 1;
                    }
                    b't' => {
                        result.push(b'\t');
                        i += 1;
                    }
                    b'b' => {
                        result.push(8); // backspace
                        i += 1;
                    }
                    b'f' => {
                        result.push(12); // form feed
                        i += 1;
                    }
                    b'(' => {
                        result.push(b'(');
                        i += 1;
                    }
                    b')' => {
                        result.push(b')');
                        i += 1;
                    }
                    b'\\' => {
                        result.push(b'\\');
                        i += 1;
                    }
                    c if c.is_ascii_digit() => {
                        // Octal escape
                        let mut octal = (c - b'0') as u16;
                        i += 1;
                        for _ in 0..2 {
                            if i < len && data[i].is_ascii_digit() && data[i] <= b'7' {
                                octal = octal * 8 + (data[i] - b'0') as u16;
                                i += 1;
                            } else {
                                break;
                            }
                        }
                        result.push(octal as u8);
                    }
                    b'\r' => {
                        // Backslash + CR (+ optional LF) = line continuation
                        i += 1;
                        if i < len && data[i] == b'\n' {
                            i += 1;
                        }
                    }
                    b'\n' => {
                        // Backslash + LF = line continuation
                        i += 1;
                    }
                    _ => {
                        // Unknown escape, just include the character
                        result.push(data[i]);
                        i += 1;
                    }
                }
            }
            _ => {
                result.push(data[i]);
                i += 1;
            }
        }
    }

    (PdfValue::Str(result), i)
}

/// Parse a hex string starting at `<`, returns (PdfValue, next_index).
fn parse_hex_string(data: &[u8], start: usize) -> (PdfValue, usize) {
    let mut i = start + 1; // skip '<'
    let len = data.len();
    let mut hex_chars = Vec::new();

    while i < len && data[i] != b'>' {
        if !is_whitespace(data[i]) {
            hex_chars.push(data[i]);
        }
        i += 1;
    }
    if i < len {
        i += 1; // skip '>'
    }

    // Pad odd-length hex with trailing 0
    if hex_chars.len() % 2 != 0 {
        hex_chars.push(b'0');
    }

    let mut result = Vec::with_capacity(hex_chars.len() / 2);
    for pair in hex_chars.chunks(2) {
        let hi = hex_digit(pair[0]);
        let lo = hex_digit(pair[1]);
        result.push((hi << 4) | lo);
    }

    (PdfValue::Str(result), i)
}

fn hex_digit(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => b - b'a' + 10,
        b'A'..=b'F' => b - b'A' + 10,
        _ => 0,
    }
}

/// Parse a name starting at `/`, returns (PdfValue, next_index).
fn parse_name(data: &[u8], start: usize) -> (PdfValue, usize) {
    let mut i = start + 1; // skip '/'
    let len = data.len();
    let mut name = Vec::new();

    while i < len && !is_whitespace(data[i]) && !is_delimiter(data[i]) {
        if data[i] == b'#' && i + 2 < len {
            // Hex escape in name
            let hi = hex_digit(data[i + 1]);
            let lo = hex_digit(data[i + 2]);
            name.push((hi << 4) | lo);
            i += 3;
        } else {
            name.push(data[i]);
            i += 1;
        }
    }

    (PdfValue::Name(name), i)
}

/// Parse a number (integer or real), returns (PdfValue, next_index).
fn parse_number(data: &[u8], start: usize) -> (PdfValue, usize) {
    let mut i = start;
    let len = data.len();
    let mut has_dot = false;

    // Sign
    if i < len && (data[i] == b'-' || data[i] == b'+') {
        i += 1;
    }

    // Check for leading dot
    if i < len && data[i] == b'.' {
        has_dot = true;
        i += 1;
    }

    while i < len && (data[i].is_ascii_digit() || data[i] == b'.') {
        if data[i] == b'.' {
            has_dot = true;
        }
        i += 1;
    }

    let s = std::str::from_utf8(&data[start..i]).unwrap_or("0");

    if has_dot {
        let val: f32 = s.parse().unwrap_or(0.0);
        (PdfValue::Real(val), i)
    } else {
        let val: i64 = s.parse().unwrap_or(0);
        (PdfValue::Integer(val), i)
    }
}

/// Parse an array starting at `[`, returns (PdfValue, next_index).
fn parse_array(data: &[u8], start: usize) -> (PdfValue, usize) {
    let mut i = start + 1; // skip '['
    let len = data.len();
    let mut elements = Vec::new();

    while i < len {
        // Skip whitespace
        if is_whitespace(data[i]) {
            i += 1;
            continue;
        }

        if data[i] == b']' {
            i += 1;
            break;
        }

        // Literal string
        if data[i] == b'(' {
            let (val, next) = parse_literal_string(data, i);
            elements.push(val);
            i = next;
            continue;
        }

        // Hex string
        if data[i] == b'<' && i + 1 < len && data[i + 1] != b'<' {
            let (val, next) = parse_hex_string(data, i);
            elements.push(val);
            i = next;
            continue;
        }

        // Nested array
        if data[i] == b'[' {
            let (val, next) = parse_array(data, i);
            elements.push(val);
            i = next;
            continue;
        }

        // Name
        if data[i] == b'/' {
            let (val, next) = parse_name(data, i);
            elements.push(val);
            i = next;
            continue;
        }

        // Number
        if data[i] == b'-' || data[i] == b'+' || data[i] == b'.' || data[i].is_ascii_digit() {
            let (val, next) = parse_number(data, i);
            elements.push(val);
            i = next;
            continue;
        }

        // Keywords (true/false/null) inside arrays
        if data[i].is_ascii_alphabetic() {
            let token_start = i;
            while i < len && data[i].is_ascii_alphabetic() {
                i += 1;
            }
            let token = std::str::from_utf8(&data[token_start..i]).unwrap_or("");
            match token {
                "true" | "false" | "null" => {
                    elements.push(PdfValue::Other);
                }
                _ => {
                    // Names without / in arrays are unusual; treat as Other
                    elements.push(PdfValue::Other);
                }
            }
            continue;
        }

        // Skip unknown
        i += 1;
    }

    (PdfValue::Array(elements), i)
}

/// Skip an inline image block. Called right after consuming `BI`.
/// We need to find `ID` (image data marker), then scan for `EI`.
fn skip_inline_image(data: &[u8], start: usize) -> usize {
    let len = data.len();
    let mut i = start;

    // Find `ID` marker (must be preceded by whitespace)
    while i + 1 < len {
        if data[i] == b'I' && data[i + 1] == b'D' {
            // Check that ID is preceded by whitespace (or start)
            let preceded = i == 0 || is_whitespace(data[i - 1]);
            if preceded {
                i += 2; // skip "ID"
                // Skip the single whitespace after ID
                if i < len && is_whitespace(data[i]) {
                    i += 1;
                }
                break;
            }
        }
        i += 1;
    }

    // Now scan for `EI` preceded by whitespace and followed by whitespace/EOF
    while i + 1 < len {
        if data[i] == b'E'
            && data[i + 1] == b'I'
            && (i == 0 || is_whitespace(data[i - 1]))
            && (i + 2 >= len || is_whitespace(data[i + 2]) || is_delimiter(data[i + 2]))
        {
            return i + 2; // past "EI"
        }
        i += 1;
    }

    // If we never found EI, return end of data
    len
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_text_ops() {
        let data = b"BT /F1 12 Tf 100 700 Td (Hello World) Tj ET";
        let ops = parse_content_stream(data).unwrap();
        assert_eq!(ops.len(), 5);
        assert_eq!(ops[0].operator, "BT");
        assert_eq!(ops[0].operands.len(), 0);
        assert_eq!(ops[1].operator, "Tf");
        assert_eq!(ops[1].operands.len(), 2);
        assert_eq!(ops[2].operator, "Td");
        assert_eq!(ops[3].operator, "Tj");
        assert_eq!(ops[4].operator, "ET");
    }

    #[test]
    fn test_parse_tj_array() {
        let data = b"BT [(Hello) -100 (World)] TJ ET";
        let ops = parse_content_stream(data).unwrap();
        assert_eq!(ops.len(), 3); // BT, TJ, ET
        let tj = &ops[1];
        assert_eq!(tj.operator, "TJ");
        assert_eq!(tj.operands.len(), 1); // one array operand
    }

    #[test]
    fn test_parse_graphics_ops() {
        let data = b"q 1 0 0 1 72 720 cm Q";
        let ops = parse_content_stream(data).unwrap();
        assert_eq!(ops[0].operator, "q");
        assert_eq!(ops[1].operator, "cm");
        assert_eq!(ops[1].operands.len(), 6);
        assert_eq!(ops[2].operator, "Q");
    }

    #[test]
    fn test_parse_real_numbers() {
        let data = b"0.5 0.5 0.5 rg";
        let ops = parse_content_stream(data).unwrap();
        assert_eq!(ops[0].operator, "rg");
        assert_eq!(ops[0].operands.len(), 3);
    }

    #[test]
    fn test_empty_content_stream() {
        let ops = parse_content_stream(b"").unwrap();
        assert!(ops.is_empty());
    }

    #[test]
    fn test_tstar_operator() {
        let data = b"BT T* ET";
        let ops = parse_content_stream(data).unwrap();
        assert_eq!(ops[1].operator, "T*");
    }
}
