use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum PackageStatus {
    Fresh,
    Stale,
    Missing,
}

pub fn package_status(
    source_fingerprint: &str,
    recorded_fingerprint: Option<&str>,
    artifacts_present: bool,
) -> PackageStatus {
    if !artifacts_present || recorded_fingerprint.is_none() {
        return PackageStatus::Missing;
    }
    if recorded_fingerprint == Some(source_fingerprint) && !source_fingerprint.is_empty() {
        PackageStatus::Fresh
    } else {
        PackageStatus::Stale
    }
}

pub fn source_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    walk(root, &mut files);
    files.sort();
    files
}

pub fn fingerprint_tree(root: &Path) -> String {
    let mut hasher = Sha256::new();
    for path in source_files(root) {
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        hasher.update(relative.as_bytes());
        hasher.update([0]);
        let bytes = fs::read(&path).unwrap_or_else(|error| {
            panic!("read {} for web UI fingerprint: {error}", path.display());
        });
        hasher.update(bytes);
        hasher.update([0]);
    }
    hex(&hasher.finalize())
}

#[allow(dead_code)]
pub fn short_hash(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))[..16].to_owned()
}

fn walk(dir: &Path, files: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|error| {
        panic!("read {} for web UI sources: {error}", dir.display());
    });
    for entry in entries {
        let entry = entry.unwrap_or_else(|error| {
            panic!("read {} entry for web UI sources: {error}", dir.display());
        });
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name == "target" {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            walk(&path, files);
        } else {
            files.push(path);
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::{PackageStatus, fingerprint_tree, package_status};

    #[test]
    fn missing_or_changed_packages_are_not_fresh() {
        assert_eq!(package_status("abc", None, false), PackageStatus::Missing);
        assert_eq!(package_status("abc", None, true), PackageStatus::Missing);
        assert_eq!(
            package_status("abc", Some("old"), true),
            PackageStatus::Stale
        );
        assert_eq!(
            package_status("abc", Some("abc"), false),
            PackageStatus::Missing
        );
        assert_eq!(
            package_status("abc", Some("abc"), true),
            PackageStatus::Fresh
        );
        assert_eq!(package_status("", Some(""), true), PackageStatus::Stale);
    }

    #[test]
    fn fingerprint_tracks_source_bytes() {
        let root = std::env::temp_dir().join(format!("web-ui-fp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/lib.rs"), "fn alpha() {}\n").unwrap();
        let first = fingerprint_tree(&root);
        std::fs::write(root.join("src/lib.rs"), "fn beta() {}\n").unwrap();
        let second = fingerprint_tree(&root);
        assert_ne!(first, second);
        let _ = std::fs::remove_dir_all(&root);
    }
}
