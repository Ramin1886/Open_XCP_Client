// SPDX-License-Identifier: GPL-2.0-only
//! Fast zero-allocation tokenizer for ASAM MCD-2 MC (A2L) files.

/// A single lexical token from an A2L file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum A2lToken<'a> {
    /// `/begin` keyword
    Begin,
    /// `/end` keyword
    End,
    /// Quoted string literal (with surrounding quotes stripped)
    Quoted(&'a str),
    /// Bare symbol, keyword, or numeric literal
    Symbol(&'a str),
}

impl<'a> A2lToken<'a> {
    /// Returns the inner string slice of the token.
    pub fn as_str(&self) -> &'a str {
        match self {
            Self::Begin => "/begin",
            Self::End => "/end",
            Self::Quoted(s) | Self::Symbol(s) => s,
        }
    }
}

/// Tokenizes an entire A2L text buffer, stripping C-style (`/* ... */`) and C++-style (`// ...`)
/// comments while preserving quoted string literals.
pub fn tokenize_a2l(input: &str) -> Vec<A2lToken<'_>> {
    let mut tokens = Vec::with_capacity(input.len() / 6);
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        let b = bytes[i];
        // Skip whitespace
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        // Skip block comments /* ... */
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(len);
            continue;
        }

        // Skip line comments // ...
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            i += 2;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        // Quoted string "..."
        if b == b'"' {
            i += 1;
            let start = i;
            while i < len {
                if bytes[i] == b'\\' && i + 1 < len {
                    i += 2;
                    continue;
                }
                if bytes[i] == b'"' {
                    break;
                }
                i += 1;
            }
            let slice = &input[start..i];
            tokens.push(A2lToken::Quoted(slice));
            if i < len && bytes[i] == b'"' {
                i += 1;
            }
            continue;
        }

        // Bare symbol or keyword
        let start = i;
        while i < len && !bytes[i].is_ascii_whitespace() && bytes[i] != b'"' {
            if bytes[i] == b'/' && i + 1 < len && (bytes[i + 1] == b'*' || bytes[i + 1] == b'/') {
                break;
            }
            i += 1;
        }
        let sym = &input[start..i];
        if sym.eq_ignore_ascii_case("/begin") {
            tokens.push(A2lToken::Begin);
        } else if sym.eq_ignore_ascii_case("/end") {
            tokens.push(A2lToken::End);
        } else {
            tokens.push(A2lToken::Symbol(sym));
        }
    }

    tokens
}

/// Parses an integer or hexadecimal (`0x...`) literal into `u64`.
pub fn parse_u64_literal(s: &str) -> Option<u64> {
    let trimmed = s.trim();
    if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        u64::from_str_radix(hex, 16).ok()
    } else {
        trimmed.parse::<u64>().ok()
    }
}

/// Parses an integer or hexadecimal (`0x...`) literal into `i64`.
pub fn parse_i64_literal(s: &str) -> Option<i64> {
    let trimmed = s.trim();
    if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        i64::from_str_radix(hex, 16).ok()
    } else {
        trimmed.parse::<i64>().ok()
    }
}
