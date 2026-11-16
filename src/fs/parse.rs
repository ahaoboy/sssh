//! Parsers for the `KEY=value` markers emitted by the shell probes in
//! `src/fs/scripts/`.
//!
//! Keeping the parsing separate from the probe scripts lets the Rust side be
//! tested without a live SSH server (see the unit tests below).

/// Value of a `KEY=value` line (trimmed), if present.
pub(crate) fn marker<'a>(stdout: &'a str, key: &str) -> Option<&'a str> {
    stdout.lines().find_map(|line| {
        let (name, value) = line.split_once('=')?;
        (name.trim() == key).then(|| value.trim())
    })
}

/// Numeric value of a `KEY=value` line, if present and parsable.
pub(crate) fn num(stdout: &str, key: &str) -> Option<u64> {
    marker(stdout, key).and_then(|value| value.parse().ok())
}

/// Whether a bare marker line (e.g. `EXISTS=yes`) is present.
pub(crate) fn flag(stdout: &str, key: &str, value: &str) -> bool {
    marker(stdout, key) == Some(value)
}

/// Extract `(total, available)` bytes from `df -Pk` output.
///
/// `-P` (POSIX) keeps each filesystem on one line, but BusyBox and some
/// legacy systems still wrap long device names onto their own line. Both
/// layouts are recognised by locating the `Capacity` column (the field
/// ending in `%`):
///
/// * `Filesystem Blocks Used Available Capacity Mount` (normal row)
/// * `Blocks Used Available Capacity Mount` (wrapped remainder)
///
/// Lines without a capacity field (header, errors) are skipped, scanning
/// from the bottom so the last filesystem wins. BusyBox prints only the
/// header when the path is missing, which yields `None`.
pub(crate) fn parse_df(stdout: &str) -> Option<(u64, u64)> {
    stdout.lines().rev().find_map(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let kb = |value: &str| value.parse::<u64>().ok();

        // Normal row: fields[4] is the capacity column.
        if fields.len() >= 5 && fields[4].ends_with('%') {
            return Some((
                kb(fields[1])?.saturating_mul(1024),
                kb(fields[3])?.saturating_mul(1024),
            ));
        }

        // Wrapped row without the filesystem name: fields[3] is capacity.
        if fields.len() >= 4 && fields[3].ends_with('%') {
            return Some((
                kb(fields[0])?.saturating_mul(1024),
                kb(fields[2])?.saturating_mul(1024),
            ));
        }

        None
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_markers() {
        let out = "EXISTS=yes\nTOTAL_BLOCKS=10\nPROBE=/home/u\n";
        assert_eq!(marker(out, "PROBE"), Some("/home/u"));
        assert_eq!(num(out, "TOTAL_BLOCKS"), Some(10));
        assert!(flag(out, "EXISTS", "yes"));
        assert!(!flag(out, "EXISTS", "no"));
        assert_eq!(marker(out, "MISSING"), None);
    }

    #[test]
    fn parses_df_row() {
        let out = "Filesystem 1024-blocks Used Available Capacity Mounted on\n\
                   /dev/sda1 1000 400 600 40% /\n";
        assert_eq!(parse_df(out), Some((1000 * 1024, 600 * 1024)));
    }

    #[test]
    fn parses_wrapped_df_row() {
        // Long device names wrap onto their own line; the numeric row is what
        // matters and must be found by scanning from the bottom.
        let out = "Filesystem 1024-blocks Used Available Capacity Mounted on\n\
                   /dev/mapper/very-long-volume-name\n\
                   1000 400 600 40% /\n";
        assert_eq!(parse_df(out), Some((1000 * 1024, 600 * 1024)));
    }

    #[test]
    fn busybox_header_only_is_unknown() {
        let out = "Filesystem 1024-blocks Used Available Capacity Mounted on\n";
        assert_eq!(parse_df(out), None);
    }

    #[test]
    fn skips_noise_lines() {
        let out = "df: /missing: No such file or directory\n\
                   Filesystem 1024-blocks Used Available Capacity Mounted on\n\
                   /dev/sda1 1000 400 600 40% /\n";
        assert_eq!(parse_df(out), Some((1000 * 1024, 600 * 1024)));
    }
}
