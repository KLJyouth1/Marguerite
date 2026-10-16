use super::OpsError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub size_bytes: u64,
}
impl FileEntry { pub fn new(path: impl Into<String>, size_bytes: u64) -> Self { Self { path: path.into(), size_bytes } } }

pub fn limit_files(mut entries: Vec<FileEntry>, max_entries: usize, max_bytes: usize) -> Result<Vec<FileEntry>, OpsError> {
    if entries.len() > max_entries { return Err(OpsError::invalid("file entry limit exceeded")); }
    let total: u64 = entries.iter().map(|e| e.size_bytes).sum();
    if total > max_bytes as u64 { return Err(OpsError::invalid("file byte limit exceeded")); }
    entries.sort_by(|a,b| a.path.cmp(&b.path));
    Ok(entries)
}
