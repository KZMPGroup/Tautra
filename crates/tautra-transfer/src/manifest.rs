use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use tautra_proto::share::FileEntry;

use crate::error::TransferError;

pub const MAX_ENTRIES: usize = 20_000;

#[derive(Debug, Clone, Default)]
pub struct Manifest {
    pub entries: Vec<FileEntry>,
    pub sources: Vec<PathBuf>,
}

fn modified_ms(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn unique_name(name: &str, taken: &[String]) -> String {
    if !taken.iter().any(|t| t.eq_ignore_ascii_case(name)) {
        return name.to_string();
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (2..)
        .map(|n| format!("{stem} ({n}){ext}"))
        .find(|candidate| !taken.iter().any(|t| t.eq_ignore_ascii_case(candidate)))
        .unwrap_or_else(|| name.to_string())
}

impl Manifest {
    fn push(&mut self, entry: FileEntry, source: PathBuf) -> Result<(), TransferError> {
        if self.entries.len() >= MAX_ENTRIES {
            return Err(TransferError::TooMany(self.entries.len() + 1));
        }
        self.entries.push(entry);
        self.sources.push(source);
        Ok(())
    }

    fn walk(&mut self, dir: &Path, prefix: &str) -> Result<(), TransferError> {
        let mut children: Vec<(String, PathBuf)> = std::fs::read_dir(dir)
            .map_err(|e| TransferError::io(dir, e))?
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                let name = entry.file_name().into_string().ok()?;
                Some((name, entry.path()))
            })
            .collect();
        children.sort_by_key(|c| c.0.to_lowercase());
        for (name, path) in children {
            self.add(&path, format!("{prefix}/{name}"))?;
        }
        Ok(())
    }

    fn add(&mut self, path: &Path, relative: String) -> Result<(), TransferError> {
        let link = std::fs::symlink_metadata(path).map_err(|e| TransferError::io(path, e))?;
        let meta = if link.file_type().is_symlink() {
            match std::fs::metadata(path) {
                Ok(meta) => meta,
                Err(e) => {
                    tracing::warn!(path = %path.display(), "symlink rusak dilewati: {e}");
                    return Ok(());
                }
            }
        } else {
            link.clone()
        };
        if meta.is_dir() {
            self.push(
                FileEntry {
                    path: relative.clone(),
                    size: 0,
                    dir: true,
                    modified_ms: modified_ms(&meta),
                },
                path.to_path_buf(),
            )?;
            if link.file_type().is_symlink() {
                tracing::warn!(path = %path.display(), "isi folder symlink tidak ikut dikirim");
                return Ok(());
            }
            self.walk(path, &relative)
        } else {
            self.push(
                FileEntry {
                    path: relative,
                    size: meta.len(),
                    dir: false,
                    modified_ms: modified_ms(&meta),
                },
                path.to_path_buf(),
            )
        }
    }
}

pub fn build_manifest(paths: &[PathBuf]) -> Result<Manifest, TransferError> {
    let mut manifest = Manifest::default();
    let mut taken: Vec<String> = Vec::new();
    for path in paths {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            tracing::warn!(path = %path.display(), "item tanpa nama dilewati");
            continue;
        };
        let name = unique_name(name, &taken);
        taken.push(name.clone());
        manifest.add(path, name)?;
    }
    if manifest.entries.is_empty() {
        return Err(TransferError::Empty);
    }
    Ok(manifest)
}
