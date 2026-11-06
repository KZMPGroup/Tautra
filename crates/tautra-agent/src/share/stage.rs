use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, SystemTime};

use tautra_clipboard::DropFiles;
use tautra_proto::share::FileEntry;
use tokio::runtime::Handle;

use crate::app::Agent;

const KEEP: Duration = Duration::from_secs(24 * 3600);

struct Item {
    index: u32,
    path: PathBuf,
    entry: FileEntry,
}

pub struct DropStage {
    agent: Weak<Agent>,
    runtime: Handle,
    offer_id: u64,
    root: PathBuf,
    items: Vec<Item>,
    requested: AtomicBool,
    shell: Mutex<Option<bool>>,
    filled: Mutex<bool>,
}

fn base_dir() -> Option<PathBuf> {
    tautra_common::paths::local_dir().ok().map(|dir| dir.join("drop"))
}

fn sweep(base: &Path) {
    let Ok(list) = fs::read_dir(base) else {
        return;
    };
    for item in list.flatten() {
        let old = item
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > KEEP);
        if old {
            if let Err(e) = fs::remove_dir_all(item.path()) {
                tracing::debug!("folder drop lama gagal dihapus: {e}");
            }
        }
    }
}

impl DropStage {
    pub fn new(agent: &Arc<Agent>, runtime: Handle, offer_id: u64, entries: &[FileEntry]) -> Option<Arc<Self>> {
        let base = base_dir()?;
        let sweep_base = base.clone();
        runtime.spawn_blocking(move || sweep(&sweep_base));
        let mut items = Vec::with_capacity(entries.len());
        for (index, entry) in entries.iter().enumerate() {
            match tautra_transfer::safe_relative(&entry.path) {
                Some(path) => items.push(Item {
                    index: index as u32,
                    path,
                    entry: entry.clone(),
                }),
                None => tracing::warn!(path = %entry.path, "nama file dari perangkat lain tidak aman, dilewati"),
            }
        }
        if items.is_empty() {
            return None;
        }
        Some(Arc::new(Self {
            agent: Arc::downgrade(agent),
            runtime,
            offer_id,
            root: base.join(offer_id.to_string()),
            items,
            requested: AtomicBool::new(false),
            shell: Mutex::new(None),
            filled: Mutex::new(false),
        }))
    }

    pub fn fill(&self) -> Result<(), String> {
        let shell = tautra_clipboard::shell_target();
        *self.shell.lock().unwrap_or_else(|e| e.into_inner()) = Some(shell);
        if shell || !self.requested.load(Ordering::Acquire) {
            return Ok(());
        }
        let mut filled = self.filled.lock().unwrap_or_else(|e| e.into_inner());
        if *filled {
            return Ok(());
        }
        let agent = self
            .agent
            .upgrade()
            .ok_or_else(|| "agent Tautra sedang berhenti".to_string())?;
        if agent.share.connection().is_none() {
            return Err("perangkat lain tidak tersambung".into());
        }
        tracing::info!(
            offer = self.offer_id,
            items = self.items.len(),
            "file dari perangkat lain diunduh dulu untuk aplikasi tujuan"
        );
        for item in &self.items {
            let path = self.root.join(&item.path);
            if item.entry.dir {
                fs::create_dir_all(&path)
                    .map_err(|e| format!("folder {} gagal dibuat: {e}", path.display()))?;
                continue;
            }
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("folder {} gagal dibuat: {e}", parent.display()))?;
            }
            self.download(&agent, item, &path)?;
        }
        *filled = true;
        tracing::info!(offer = self.offer_id, root = %self.root.display(), "file dari perangkat lain siap di-drop");
        Ok(())
    }

    fn download(&self, agent: &Arc<Agent>, item: &Item, path: &Path) -> Result<(), String> {
        let mut reader = tautra_transfer::stream_file(
            &self.runtime,
            agent.share.connections(),
            self.offer_id,
            item.index,
            item.entry.size,
        );
        let mut file =
            fs::File::create(path).map_err(|e| format!("file {} gagal dibuat: {e}", path.display()))?;
        let mut buf = vec![0u8; tautra_transfer::CHUNK];
        let mut written = 0u64;
        loop {
            let n = reader
                .read(&mut buf)
                .map_err(|e| format!("{} gagal diunduh: {e}", item.entry.path))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])
                .map_err(|e| format!("file {} gagal ditulis: {e}", path.display()))?;
            written += n as u64;
        }
        if written != item.entry.size {
            return Err(format!(
                "{} tidak lengkap ({written} dari {} byte)",
                item.entry.path, item.entry.size
            ));
        }
        let modified = SystemTime::UNIX_EPOCH + Duration::from_millis(item.entry.modified_ms);
        let _ = file.set_modified(modified);
        Ok(())
    }
}

impl DropFiles for DropStage {
    fn paths(&self) -> Result<Vec<PathBuf>, String> {
        fs::create_dir_all(&self.root)
            .map_err(|e| format!("folder {} gagal dibuat: {e}", self.root.display()))?;
        let mut tops = Vec::new();
        for item in &self.items {
            if item.path.components().count() != 1 {
                continue;
            }
            let path = self.root.join(&item.path);
            let made = if item.entry.dir {
                fs::create_dir_all(&path)
            } else if path.exists() {
                Ok(())
            } else {
                fs::File::create(&path).map(|_| ())
            };
            made.map_err(|e| format!("{} gagal dibuat: {e}", path.display()))?;
            tops.push(path);
        }
        if tops.is_empty() {
            return Err("tidak ada file untuk di-drop".into());
        }
        self.requested.store(true, Ordering::Release);
        Ok(tops)
    }

    fn dropped_on_shell(&self) -> Option<bool> {
        *self.shell.lock().unwrap_or_else(|e| e.into_inner())
    }
}
