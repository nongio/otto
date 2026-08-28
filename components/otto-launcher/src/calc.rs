//! Arithmetic typed into the query field.
//!
//! `24.5*3`, `3+100`, `1000,00/45.3` — when what has been typed is an
//! expression rather than a search, the answer is offered as the first row.
//! It is not ranked against the query the way the other sources are: the
//! answer to `24.5*3` is `73.5`, which shares nothing with what was typed and
//! would be filtered straight out.
//!
//! Only whole expressions count. A query has to parse from end to end and
//! contain at least one operator, so `3` on its own is a search for something
//! called "3" and `code` is not an unfinished sum.
//!
//! **Decimal separators.** A comma is a decimal point: `1000,00` is a thousand.
//! When a number carries both separators the last one is the decimal point and
//! the other is grouping, so `1.000,50` and `1,000.50` both read as a thousand
//! and a half.

use std::process::Command;

use crate::source::{Item, Origin, Source};

pub struct Calculator {
    index: usize,
    /// The last answer worked out, so activating the row knows what to copy.
    last: Option<String>,
}

impl Calculator {
    pub fn new(index: usize) -> Self {
        Self { index, last: None }
    }
}

impl Source for Calculator {
    fn label(&self) -> &'static str {
        otto_kit::t!("launcher-badge-calc")
    }

    /// Nothing to browse: an answer exists only for a question.
    fn items(&mut self) -> Vec<Item> {
        Vec::new()
    }

    fn answer(&mut self, query: &str) -> Option<Item> {
        let answer = evaluate(query)?;
        self.last = Some(answer.clone());
        Some(Item {
            title: answer,
            subtitle: Some(format!("{} — copy to the clipboard", query.trim())),
            icon: icon_name(),
            search_terms: Vec::new(),
            origin: Origin {
                source: self.index,
                index: 0,
            },
        })
    }

    fn activate(&mut self, _index: usize) -> Result<(), String> {
        let answer = self.last.as_ref().ok_or("nothing to copy")?;
        // `wl-copy` forks and stays alive to serve the selection, which this
        // process cannot: it is about to exit, and a Wayland clipboard offer
        // dies with the client that made it.
        Command::new("wl-copy")
            .arg(answer)
            .spawn()
            .map(|_| ())
            .map_err(|err| format!("could not copy the answer: {err}"))
    }
}

/// The first calculator icon the theme actually has.
///
/// The generic freedesktop name is not in every theme — this one is not in
/// ours — so the installed calculator's own icon is the fallback, and a blank
/// square is the last resort.
fn icon_name() -> Option<String> {
    static NAME: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    NAME.get_or_init(|| {
        [
            "accessories-calculator",
            "org.gnome.Calculator",
            "gnome-calculator",
            "galculator",
        ]
        .into_iter()
        .find(|name| otto_kit::icons::find_icon(name, 56, 2).is_some())
        .map(str::to_string)
    })
    .clone()
}

/// Work out `input`, or `None` if it is not an expression.
pub fn evaluate(input: &str) -> Option<String> {
    let tokens = tokenize(input)?;
    if !tokens.iter().any(|token| matches!(token, Token::Op(_))) {
        // A bare number is a search, not a sum.
        return None;
    }

    let mut parser = Parser {
        tokens: &tokens,
        at: 0,
    };
    let value = parser.expression()?;
    if parser.at != tokens.len() {
        return None;
    }
    if !value.is_finite() {
        return None;
    }

    // Answer in the notation the question was asked in.
    let comma = input.contains(',') && !input.contains('.');
    Some(format(value, comma))
}

fn format(value: f64, comma: bool) -> String {
    let mut text = if value == value.trunc() && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        // Ten decimal places is past the noise of binary floating point —
        // 0.1 + 0.2 rounds back to 0.3 — and the zeros go afterwards.
        let mut rounded = format!("{value:.10}");
        while rounded.ends_with('0') {
            rounded.pop();
        }
        if rounded.ends_with('.') {
            rounded.pop();
        }
        rounded
    };
    if comma {
        text = text.replace('.', ",");
    }
    text
}

// ---------------------------------------------------------------------------
// Tokens
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    Op(char),
    Open,
    Close,
}

fn tokenize(input: &str) -> Option<Vec<Token>> {
    let chars: Vec<char> = input.trim().chars().collect();
    if chars.is_empty() {
        return None;
    }

    let mut tokens = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        let c = chars[at];
        if c.is_whitespace() {
            at += 1;
            continue;
        }
        if c.is_ascii_digit() || c == '.' || c == ',' {
            let start = at;
            while at < chars.len()
                && (chars[at].is_ascii_digit() || chars[at] == '.' || chars[at] == ',')
            {
                at += 1;
            }
            let text: String = chars[start..at].iter().collect();
            tokens.push(Token::Number(parse_number(&text)?));
            continue;
        }
        at += 1;
        match c {
            '(' => tokens.push(Token::Open),
            ')' => tokens.push(Token::Close),
            '+' | '-' | '*' | '/' | '%' | '^' => tokens.push(Token::Op(c)),
            // The characters a keyboard layout or a paste can produce for the
            // same two operations.
            '×' | 'x' | '·' => tokens.push(Token::Op('*')),
            '÷' | ':' => tokens.push(Token::Op('/')),
            '−' => tokens.push(Token::Op('-')),
            // Anything else means this was never an expression.
            _ => return None,
        }
    }
    Some(tokens)
}

/// Read one number, working out what its separators mean.
fn parse_number(text: &str) -> Option<f64> {
    let dot = text.rfind('.');
    let comma = text.rfind(',');

    let normalized = match (dot, comma) {
        // Both: whichever comes last is the decimal point, the other groups.
        (Some(d), Some(c)) if d > c => text.replace(',', ""),
        (Some(_), Some(_)) => text.replace('.', "").replace(',', "."),
        // A comma alone is a decimal point — `1000,00` is a thousand.
        (None, Some(_)) => text.replace(',', "."),
        _ => text.to_string(),
    };
    // One separator at most survives, or this was never a number.
    if normalized.matches('.').count() > 1 {
        return None;
    }
    normalized.parse().ok()
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

struct Parser<'a> {
    tokens: &'a [Token],
    at: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn eat_op(&mut self, wanted: &[char]) -> Option<char> {
        match self.peek() {
            Some(Token::Op(op)) if wanted.contains(op) => {
                let op = *op;
                self.at += 1;
                Some(op)
            }
            _ => None,
        }
    }

    fn expression(&mut self) -> Option<f64> {
        let mut value = self.term()?;
        while let Some(op) = self.eat_op(&['+', '-']) {
            let rhs = self.term()?;
            value = if op == '+' { value + rhs } else { value - rhs };
        }
        Some(value)
    }

    fn term(&mut self) -> Option<f64> {
        let mut value = self.power()?;
        while let Some(op) = self.eat_op(&['*', '/', '%']) {
            let rhs = self.power()?;
            value = match op {
                '*' => value * rhs,
                '/' => value / rhs,
                _ => value % rhs,
            };
        }
        Some(value)
    }

    /// Right-associative, so `2^3^2` is 512 rather than 64.
    fn power(&mut self) -> Option<f64> {
        let base = self.unary()?;
        if self.eat_op(&['^']).is_some() {
            let exponent = self.power()?;
            return Some(base.powf(exponent));
        }
        Some(base)
    }

    fn unary(&mut self) -> Option<f64> {
        if let Some(op) = self.eat_op(&['-', '+']) {
            let value = self.unary()?;
            return Some(if op == '-' { -value } else { value });
        }
        self.primary()
    }

    fn primary(&mut self) -> Option<f64> {
        match self.peek()? {
            Token::Number(value) => {
                let value = *value;
                self.at += 1;
                Some(value)
            }
            Token::Open => {
                self.at += 1;
                let value = self.expression()?;
                match self.peek() {
                    Some(Token::Close) => {
                        self.at += 1;
                        Some(value)
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works_out_the_examples() {
        assert_eq!(evaluate("24.5*3").as_deref(), Some("73.5"));
        assert_eq!(evaluate("3+100").as_deref(), Some("103"));
        assert_eq!(evaluate("1000,00/45.3").as_deref(), Some("22.0750551876"));
    }

    #[test]
    fn a_comma_is_a_decimal_point() {
        assert_eq!(evaluate("1000,00/2").as_deref(), Some("500"));
        assert_eq!(evaluate("1,5+1,5").as_deref(), Some("3"));
    }

    #[test]
    fn an_answer_is_written_the_way_the_question_was() {
        assert_eq!(evaluate("1,5*3").as_deref(), Some("4,5"));
        assert_eq!(evaluate("1.5*3").as_deref(), Some("4.5"));
    }

    #[test]
    fn both_separators_means_the_last_one_is_the_decimal_point() {
        assert_eq!(parse_number("1.000,50"), Some(1000.5));
        assert_eq!(parse_number("1,000.50"), Some(1000.5));
    }

    #[test]
    fn precedence_and_parentheses_hold() {
        assert_eq!(evaluate("2+3*4").as_deref(), Some("14"));
        assert_eq!(evaluate("(2+3)*4").as_deref(), Some("20"));
        assert_eq!(evaluate("2^3^2").as_deref(), Some("512"));
        assert_eq!(evaluate("-3+10").as_deref(), Some("7"));
    }

    #[test]
    fn floating_point_noise_is_rounded_away() {
        assert_eq!(evaluate("0.1+0.2").as_deref(), Some("0.3"));
    }

    #[test]
    fn a_search_is_not_an_expression() {
        assert_eq!(evaluate("code"), None);
        assert_eq!(evaluate("3"), None, "a bare number is a search");
        assert_eq!(evaluate(""), None);
        assert_eq!(evaluate("2+"), None, "an unfinished sum has no answer");
        assert_eq!(evaluate("gimp 2"), None);
        assert_eq!(evaluate("(2+3"), None);
    }

    #[test]
    fn dividing_by_zero_has_no_answer_to_show() {
        assert_eq!(evaluate("1/0"), None);
    }
}
