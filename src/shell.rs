//! POSIX shell helpers shared by the CLI and the filesystem layer.
//!
//! Remote commands are built as a single command line and handed to the
//! server, so every user-provided value (paths, script text) must be quoted
//! before interpolation.

/// Quote a value as a single POSIX shell argument.
///
/// Always wraps the value in single quotes and escapes embedded single
/// quotes as `'\''`, which works for every POSIX shell (`sh`, `dash`,
/// BusyBox `ash`, bash, zsh, …).
///
/// ```
/// assert_eq!(sssh::shell_quote("a b"), "'a b'");
/// assert_eq!(sssh::shell_quote("it's"), r"'it'\''s'");
/// ```
pub fn shell_quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('\'');
    for ch in value.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_plain_values() {
        assert_eq!(shell_quote("abc"), "'abc'");
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn escapes_single_quotes() {
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
        assert_eq!(shell_quote("'"), r"''\'''");
    }

    #[test]
    fn preserves_special_characters() {
        assert_eq!(shell_quote("$HOME `id`"), "'$HOME `id`'");
        assert_eq!(shell_quote("a\nb"), "'a\nb'");
    }
}
