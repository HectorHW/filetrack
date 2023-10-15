use std::{
    io,
    os::unix::prelude::MetadataExt,
    path::{Path, PathBuf},
};

/// Search for logfile and its rotated versions as `path`, `path.1`, `path.2` and so on
///
/// Returns pairs of path and inode
pub fn glob_rotated_logs(
    path: impl AsRef<Path>,
    max_depth: usize,
) -> io::Result<Vec<(PathBuf, u64)>> {
    let mut result = vec![];

    result.push((path.as_ref().to_path_buf(), get_inode_by_path(&path)?));

    for i in 1..=max_depth {
        let path = append_extension(path.as_ref().to_path_buf(), i.to_string());
        if !path.exists() {
            break;
        }
        let inode = get_inode_by_path(&path)?;
        result.push((path, inode));
    }

    result.reverse();

    Ok(result)
}

/// Ask the filesystem for metadata and return inode for fs object specified by `path`
pub fn get_inode_by_path(path: impl AsRef<Path>) -> io::Result<u64> {
    let metadata = std::fs::metadata(&path)?;
    Ok(metadata.ino())
}

/// Add extension to existing PathBuf
///
/// ## Example
///
/// ```rust
/// use std::path::PathBuf;
/// let original_path = "/var/log/mail.log".into();
/// let rotated_path = filetrack::path_utils::append_extension(original_path, "1");
/// assert_eq!(rotated_path, PathBuf::from("/var/log/mail.log.1"));
/// ```
pub fn append_extension(path: PathBuf, ext: impl AsRef<std::ffi::OsStr>) -> PathBuf {
    let mut os_string: std::ffi::OsString = path.into();
    os_string.push(".");
    os_string.push(ext.as_ref());
    os_string.into()
}
