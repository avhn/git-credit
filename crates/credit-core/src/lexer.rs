//! Language-independent lexical units for token attribution.
//!
//! This lexer deliberately does not parse a language grammar. Comments retain
//! their words, strings end at physical line boundaries, and indentation uses
//! eight-column tab stops. All slices borrow the caller's UTF-8 input.

use crate::NormalizationMode;

/// Weight of a word, number, string or indentation token, in per-mille units.
pub const TOKEN_WEIGHT: u16 = 1000;
/// Weight of punctuation, except a zero-weight trailing comma.
pub const PUNCTUATION_WEIGHT: u16 = 250;

/// Lexical category, including canonical indentation width rather than spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TokenKind {
    /// Unicode alphanumeric word, allowing underscores and dollar signs.
    Word,
    /// Number beginning with an ASCII digit, including decimal and exponent forms.
    Number,
    /// Single-, double-, or backtick-quoted text, with backslash escapes.
    String,
    /// An operator, bracket, comment marker or other non-whitespace symbol.
    Punctuation,
    /// Leading indentation in columns, with tabs advancing to eight-column stops.
    Indentation(usize),
}

/// One lexical unit and its physical, zero-based line position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token<'a> {
    /// Original token spelling. Indentation uses an empty spelling and its width
    /// in `kind`, so equivalent spaces and tabs have the same canonical key.
    pub text: &'a str,
    /// Lexical category, independent of canonical text equality.
    pub kind: TokenKind,
    /// Zero-based physical line index in the input file.
    pub line: usize,
    /// Fixed-point lexical weight: 1000, 250, or zero for a trailing comma.
    pub weight: u16,
}

impl<'a> Token<'a> {
    /// Canonical identity for interning: ordinary text uses `(None, spelling)`;
    /// indentation uses `(Some(width), "")`. Lexical category, position and
    /// weight do not affect equality. Exclude zero-weight tokens before diffing.
    pub fn key(&self) -> (Option<usize>, &'a str) {
        match self.kind {
            TokenKind::Indentation(width) => (Some(width), ""),
            _ => (None, self.text),
        }
    }
}

/// Tokenize arbitrary UTF-8 without I/O, grammar dependencies or syntax errors.
/// Whitespace is ignored except for one indentation token per nonblank line in
/// significant mode. Blank lines remain weightless. An unterminated string stops
/// at the physical line end. Comment delimiters are punctuation; comment words
/// remain separate tokens, even when the comment contains quotation marks.
///
/// A comma immediately followed by whitespace and a closing bracket has weight
/// zero. Indentation tokens do not interfere with this lookahead. Strings and
/// comments between a comma and bracket prevent the trailing-comma discount.
///
/// # Examples
/// ```
/// use credit_core::{NormalizationMode, tokenize};
/// let tokens = tokenize("call(value,\n)", NormalizationMode::IgnoreIndentation);
/// assert_eq!(tokens.iter().find(|token| token.text == ",").unwrap().weight, 0);
/// assert_eq!(tokens.last().unwrap().line, 1);
/// ```
pub fn tokenize(text: &str, mode: NormalizationMode) -> Vec<Token<'_>> {
    let mut tokens = Vec::new();
    let mut block_comment = false;
    for (line, raw) in text.lines().enumerate() {
        if raw.trim().is_empty() {
            continue;
        }
        if mode == NormalizationMode::IndentationSignificant {
            let width =
                raw.chars()
                    .take_while(|c| c.is_whitespace())
                    .fold(0_usize, |column, character| {
                        if character == '\t' {
                            column + (8 - column % 8)
                        } else {
                            column + 1
                        }
                    });
            tokens.push(Token {
                text: "",
                kind: TokenKind::Indentation(width),
                line,
                weight: TOKEN_WEIGHT,
            });
        }
        lex_line(raw, line, &mut block_comment, &mut tokens);
    }
    // Walk backward so indentation on the next physical line can be ignored
    // without allocating a second sequence or repeatedly scanning long blanks.
    let mut next = None;
    for token in tokens.iter_mut().rev() {
        if matches!(token.kind, TokenKind::Indentation(_)) {
            continue;
        }
        if token.text == "," && matches!(next, Some(")" | "]" | "}")) {
            token.weight = 0;
        }
        next = Some(token.text);
    }
    tokens
}

/// Scan one physical line while carrying only multiline-comment state forward.
fn lex_line<'a>(
    mut rest: &'a str,
    line: usize,
    block_comment: &mut bool,
    tokens: &mut Vec<Token<'a>>,
) {
    let mut line_comment = false;
    while let Some(first) = rest.chars().next() {
        if first.is_whitespace() {
            rest = &rest[first.len_utf8()..];
            continue;
        }
        let comment = *block_comment || line_comment;
        let (length, kind) = if !comment && matches!(first, '\'' | '"' | '`') {
            (string_length(rest, first), TokenKind::String)
        } else if first.is_ascii_digit() && !comment {
            (number_length(rest), TokenKind::Number)
        } else if word_character(first) {
            (
                rest.char_indices()
                    .take_while(|(_, c)| word_character(*c))
                    .map(|(offset, c)| offset + c.len_utf8())
                    .last()
                    .unwrap_or(first.len_utf8()),
                TokenKind::Word,
            )
        } else {
            (operator_length(rest), TokenKind::Punctuation)
        };
        let (spelling, remaining) = rest.split_at(length);
        if !line_comment {
            if *block_comment {
                if spelling == "*/" {
                    *block_comment = false;
                }
            } else if spelling == "/*" {
                *block_comment = true;
            } else if matches!(spelling, "//" | "#" | "--") {
                line_comment = true;
            }
        }
        tokens.push(Token {
            text: spelling,
            kind,
            line,
            weight: if kind == TokenKind::Punctuation {
                PUNCTUATION_WEIGHT
            } else {
                TOKEN_WEIGHT
            },
        });
        rest = remaining;
    }
}

/// Recognize the deliberately broad, Unicode-aware identifier alphabet.
fn word_character(character: char) -> bool {
    character.is_alphanumeric() || matches!(character, '_' | '$')
}

/// Include the closing quote, respecting escaped quotes and escaped backslashes.
fn string_length(text: &str, quote: char) -> usize {
    let mut escaped = false;
    for (offset, character) in text.char_indices().skip(1) {
        if escaped {
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == quote {
            return offset + character.len_utf8();
        }
    }
    text.len()
}

/// Keep common numeric spellings together without consuming adjacent operators.
/// Dots join a number only when followed by a digit, so ranges stay punctuation.
fn number_length(text: &str) -> usize {
    let mut characters = text.char_indices().peekable();
    let mut end = 0;
    let mut previous = None;
    while let Some((offset, character)) = characters.next() {
        let accepted = word_character(character)
            || (character == '.' && characters.peek().is_some_and(|(_, c)| c.is_ascii_digit()))
            || (matches!(character, '+' | '-')
                && matches!(previous, Some('e' | 'E'))
                && characters.peek().is_some_and(|(_, c)| c.is_ascii_digit()));
        if !accepted {
            break;
        }
        end = offset + character.len_utf8();
        previous = Some(character);
    }
    end
}

/// Longest first, including comment delimiters so their words remain independent.
fn operator_length(text: &str) -> usize {
    const OPERATORS: &[&str] = &[
        ">>>=", "<<=", ">>=", "===", "!==", "**=", "&&=", "||=", "??=", "...", "..=", "<=>", ">>>",
        "->*", "==", "!=", "<=", ">=", "=>", "->", "::", "&&", "||", "+=", "-=", "*=", "/=", "%=",
        "&=", "|=", "^=", "<<", ">>", "++", "--", "**", "??", "?.", "..", ":=", "//", "/*", "*/",
        "##", ".*",
    ];
    OPERATORS
        .iter()
        .find(|operator| text.starts_with(**operator))
        .map_or_else(
            || text.chars().next().map_or(0, char::len_utf8),
            |op| op.len(),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reduce to canonical keys while ignoring trailing-comma spelling differences.
    fn canonical(text: &str) -> Vec<(Option<usize>, String)> {
        tokenize(text, NormalizationMode::IgnoreIndentation)
            .into_iter()
            .filter(|token| token.weight > 0)
            .map(|token| (token.key().0, token.key().1.to_owned()))
            .collect()
    }

    #[test]
    fn rewrap_and_trailing_comma_preserve_meaningful_sequence() {
        assert_eq!(
            canonical("call(alpha, beta)"),
            canonical("call(\n alpha,\n beta,\n)")
        );
        for closer in [")", "]", "}"] {
            let text = format!(",\n \t{closer}");
            let tokens = tokenize(&text, NormalizationMode::IndentationSignificant);
            assert_eq!(
                tokens
                    .iter()
                    .find(|token| token.text == ",")
                    .unwrap()
                    .weight,
                0
            );
        }
        assert_eq!(
            tokenize(", /* note */ )", NormalizationMode::IgnoreIndentation)[0].weight,
            250
        );
    }

    #[test]
    fn unicode_words_numbers_and_maximal_operators() {
        let tokens = tokenize(
            "λ变量 $name_2 12.5e-3 0xff 1..=2 >>>= !== => :: && += ⊕",
            NormalizationMode::IgnoreIndentation,
        );
        let text: Vec<_> = tokens.iter().map(|token| token.text).collect();
        assert_eq!(
            text,
            [
                "λ变量", "$name_2", "12.5e-3", "0xff", "1", "..=", "2", ">>>=", "!==", "=>", "::",
                "&&", "+=", "⊕"
            ]
        );
        assert_eq!(tokens[0].kind, TokenKind::Word);
        assert_eq!(tokens[2].kind, TokenKind::Number);
        assert_eq!(tokens.last().unwrap().weight, PUNCTUATION_WEIGHT);
    }

    #[test]
    fn quotes_escapes_and_unterminated_strings_are_line_bounded() {
        let tokens = tokenize(
            "\"a \\\" b\" 'c\\\\' `d e`\n\"unterminated\\\nnext",
            NormalizationMode::IgnoreIndentation,
        );
        assert_eq!(tokens.len(), 5);
        assert!(
            tokens[..4]
                .iter()
                .all(|token| token.kind == TokenKind::String)
        );
        assert_eq!(tokens[3].text, "\"unterminated\\");
        assert_eq!(tokens[4].text, "next");
        assert_eq!(tokens[4].line, 2);
        assert_ne!(canonical("\"a b\""), canonical("\"a  b\""));
    }

    #[test]
    fn comments_keep_words_not_quoted_literals() {
        for marker in ["//", "#", "--", "/*"] {
            let text = format!("{marker} \"two words\" 123\nmore */ \"literal\"");
            let tokens = tokenize(&text, NormalizationMode::IgnoreIndentation);
            assert_eq!(tokens[0].kind, TokenKind::Punctuation);
            assert_eq!(tokens[2].text, "two");
            assert_eq!(tokens[3].text, "words");
            assert_eq!(tokens[5].kind, TokenKind::Word);
            assert_eq!(tokens.last().unwrap().kind, TokenKind::String);
        }
        let tokens = tokenize(
            "/* a\n'quoted words'\n*/ value",
            NormalizationMode::IgnoreIndentation,
        );
        assert!(!tokens.iter().any(|token| token.kind == TokenKind::String));
        assert_eq!(canonical("\"// not a comment\" value").len(), 2);
    }

    #[test]
    fn indentation_width_and_blank_lines_are_explicit() {
        let tokens = tokenize(
            "\tvalue\r\n  \r\n        value\nvalue\n",
            NormalizationMode::IndentationSignificant,
        );
        assert_eq!(tokens.len(), 6);
        assert_eq!(tokens[0].key(), tokens[2].key());
        assert_eq!(tokens[0].kind, TokenKind::Indentation(8));
        assert_eq!(tokens[2].line, 2);
        assert_eq!(tokens[4].kind, TokenKind::Indentation(0));
        assert_eq!(tokens[0].weight, TOKEN_WEIGHT);
        assert_eq!(
            tokenize(" \t\r\n\n", NormalizationMode::IndentationSignificant),
            []
        );
        assert_eq!(tokenize("", NormalizationMode::IgnoreIndentation), []);
    }
}
