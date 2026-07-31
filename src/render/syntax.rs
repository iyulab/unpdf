//! Markdown syntax helpers shared by the batch and streaming renderers.
//!
//! Both renderers produce Markdown from the same document model, so the rules for what has
//! to be escaped and how a number is spelled are properties of the output format, not of
//! either renderer. Kept in one place, a change to an escaping rule cannot reach one path
//! and miss the other — a divergence that is hard to notice, since it shows up only in table
//! cells and around emphasis characters.

/// Escape the characters that would otherwise be read as Markdown syntax.
pub(super) fn escape_markdown(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            // Core formatting that must be escaped
            '\\' | '`' | '*' | '_' |
            // Brackets for links/images, pipe for tables
            '[' | ']' | '|' => {
                result.push('\\');
                result.push(c);
            }
            // NOT escaped (only special at line start or in specific contexts):
            // '.' '-' '!' '#' '+' '>' '(' ')' '{' '}'
            _ => result.push(c),
        }
    }
    result
}

/// Spell a number as an uppercase Roman numeral, for Roman-numbered list markers.
pub(super) fn to_roman(mut num: u32) -> String {
    let numerals = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];

    let mut result = String::new();
    for (value, symbol) in numerals {
        while num >= value {
            result.push_str(symbol);
            num -= value;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_markdown() {
        assert_eq!(escape_markdown("Hello *world*"), "Hello \\*world\\*");
        assert_eq!(escape_markdown("[link]"), "\\[link\\]");
        assert_eq!(escape_markdown("a | b"), "a \\| b");
        assert_eq!(escape_markdown("snake_case"), "snake\\_case");
    }

    #[test]
    fn test_escape_markdown_leaves_line_start_syntax_alone() {
        // These are only special in a position the renderer controls, and escaping them
        // mid-sentence would put backslashes into ordinary prose.
        assert_eq!(escape_markdown("1. 2 - 3 # 4 > 5"), "1. 2 - 3 # 4 > 5");
    }

    #[test]
    fn test_to_roman() {
        assert_eq!(to_roman(1), "I");
        assert_eq!(to_roman(4), "IV");
        assert_eq!(to_roman(9), "IX");
        assert_eq!(to_roman(14), "XIV");
        assert_eq!(to_roman(2024), "MMXXIV");
    }

    #[test]
    fn test_to_roman_zero_is_empty() {
        // No Roman numeral exists for zero; the caller numbers list items from one.
        assert_eq!(to_roman(0), "");
    }
}
