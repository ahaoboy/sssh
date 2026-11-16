//! POSIX path helpers for remote paths.
//!
//! Remote paths are plain strings interpreted by the remote shell and
//! utilities, so they must be manipulated with POSIX rules even when the
//! local process runs on Windows (where [`std::path`] would produce `\`).

/// Expand `~` and relative paths against the remote home directory.
///
/// * `~` → `home`
/// * `~/x` → `home/x`
/// * `/x` → unchanged
/// * `x` → `home/x`
///
/// When `home` is empty (the probe failed) the input is returned unchanged.
pub(crate) fn expand_with_home(path: &str, home: &str) -> String {
    if home.is_empty() {
        return path.to_string();
    }
    let home = home.trim_end_matches('/');
    if path == "~" {
        return if home.is_empty() {
            "/".to_string()
        } else {
            home.to_string()
        };
    }
    if let Some(rest) = path.strip_prefix("~/") {
        return format!("{home}/{rest}");
    }
    if path.starts_with('/') {
        return path.to_string();
    }
    if path.is_empty() {
        return home.to_string();
    }
    format!("{home}/{path}")
}

/// Parent directory of a POSIX path (`/a/b` → `/a`, `/a` → `/`).
pub(crate) fn parent_dir(path: &str) -> Option<String> {
    if path == "/" {
        return None;
    }
    let path = path.trim_end_matches('/');
    path.rfind('/').map(|i| {
        if i == 0 {
            "/".to_string()
        } else {
            path[..i].to_string()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_tilde() {
        assert_eq!(expand_with_home("~", "/home/u"), "/home/u");
        assert_eq!(expand_with_home("~/a", "/home/u"), "/home/u/a");
        assert_eq!(expand_with_home("~/a", "/home/u/"), "/home/u/a");
    }

    #[test]
    fn resolves_relative_paths_against_home() {
        assert_eq!(expand_with_home("a/b", "/home/u"), "/home/u/a/b");
        assert_eq!(expand_with_home("", "/home/u"), "/home/u");
        assert_eq!(expand_with_home("./a", "/home/u"), "/home/u/./a");
    }

    #[test]
    fn keeps_absolute_paths() {
        assert_eq!(expand_with_home("/etc/hosts", "/home/u"), "/etc/hosts");
    }

    #[test]
    fn leaves_paths_alone_without_home() {
        assert_eq!(expand_with_home("~/a", ""), "~/a");
        assert_eq!(expand_with_home("a", ""), "a");
    }

    #[test]
    fn finds_parent_dirs() {
        assert_eq!(parent_dir("/a/b").as_deref(), Some("/a"));
        assert_eq!(parent_dir("/a").as_deref(), Some("/"));
        assert_eq!(parent_dir("/a/").as_deref(), Some("/"));
        assert_eq!(parent_dir("/"), None);
        assert_eq!(parent_dir("a"), None);
    }
}
