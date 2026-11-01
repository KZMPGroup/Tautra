use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tautra_common::time::unix_ms;
use tautra_ipc::api::{TransferDirection, TransferState, TransferView};
use tautra_proto::share::{ClipFormat, FileEntry};
use tokio_util::sync::CancellationToken;

const MAX_CLIP_OFFERS: usize = 4;
const MAX_PUSH_OFFERS: usize = 32;
const PUSH_OFFER_TTL: Duration = Duration::from_secs(24 * 3600);
const MAX_TRANSFERS: usize = 50;

pub struct SharedFiles {
    pub entries: Vec<FileEntry>,
    pub sources: Vec<PathBuf>,
}

pub struct SharedOffer {
    pub id: u64,
    pub push: Option<u64>,
    pub created: Instant,
    pub clip: Vec<(ClipFormat, Arc<Vec<u8>>)>,
    pub files: Option<Arc<SharedFiles>>,
    pub sent: Arc<AtomicU64>,
    pub completed: Vec<bool>,
}

pub struct FileLookup {
    pub path: PathBuf,
    pub entry: FileEntry,
    pub sent: Arc<AtomicU64>,
}

#[derive(Default)]
pub struct OfferBook {
    offers: VecDeque<SharedOffer>,
}

impl OfferBook {
    pub fn insert(&mut self, offer: SharedOffer) {
        self.offers.push_back(offer);
        let clips = self.offers.iter().filter(|o| o.push.is_none()).count();
        let mut excess = clips.saturating_sub(MAX_CLIP_OFFERS);
        self.offers.retain(|o| {
            if o.push.is_none() && excess > 0 {
                excess -= 1;
                return false;
            }
            true
        });
        let pushes = self.offers.iter().filter(|o| o.push.is_some()).count();
        let mut excess = pushes.saturating_sub(MAX_PUSH_OFFERS);
        self.offers.retain(|o| {
            if o.push.is_some() && (excess > 0 || o.created.elapsed() > PUSH_OFFER_TTL) {
                excess = excess.saturating_sub(1);
                return false;
            }
            true
        });
    }

    fn get(&self, id: u64) -> Option<&SharedOffer> {
        self.offers.iter().find(|o| o.id == id)
    }

    pub fn clip(&self, id: u64, format: ClipFormat) -> Option<Arc<Vec<u8>>> {
        self.get(id)?
            .clip
            .iter()
            .find(|(f, _)| *f == format)
            .map(|(_, data)| data.clone())
    }

    pub fn file(&self, id: u64, index: u32) -> Option<FileLookup> {
        let offer = self.get(id)?;
        let files = offer.files.as_ref()?;
        let entry = files.entries.get(index as usize)?;
        if entry.dir {
            return None;
        }
        Some(FileLookup {
            path: files.sources.get(index as usize)?.clone(),
            entry: entry.clone(),
            sent: offer.sent.clone(),
        })
    }

    pub fn complete(&mut self, id: u64, index: u32) -> Option<(u64, bool)> {
        let offer = self.offers.iter_mut().find(|o| o.id == id)?;
        let transfer = offer.push?;
        if let Some(slot) = offer.completed.get_mut(index as usize) {
            *slot = true;
        }
        Some((transfer, offer.completed.iter().all(|done| *done)))
    }

    pub fn all_completed(&self, id: u64) -> bool {
        self.get(id)
            .is_some_and(|o| o.completed.iter().all(|done| *done))
    }

    pub fn remove(&mut self, id: u64) {
        self.offers.retain(|o| o.id != id);
    }
}

pub struct Transfer {
    pub id: u64,
    pub offer_id: u64,
    pub direction: TransferDirection,
    pub peer_name: String,
    pub name: String,
    pub items: u32,
    pub files: u32,
    pub total: u64,
    pub base: Arc<AtomicU64>,
    pub current: Arc<AtomicU64>,
    pub state: TransferState,
    pub error: Option<String>,
    pub started_ms: u64,
    pub finished_ms: Option<u64>,
    pub path: Option<PathBuf>,
    pub cancel: CancellationToken,
}

impl Transfer {
    pub fn finished(&self) -> bool {
        matches!(
            self.state,
            TransferState::Done | TransferState::Failed | TransferState::Cancelled
        )
    }

    pub fn finish(&mut self, state: TransferState, error: Option<String>) {
        if self.finished() {
            return;
        }
        self.state = state;
        self.error = error;
        self.finished_ms = Some(unix_ms());
        if state == TransferState::Done {
            self.base
                .store(self.total, std::sync::atomic::Ordering::Relaxed);
            self.current.store(0, std::sync::atomic::Ordering::Relaxed);
        }
    }

    fn view(&self) -> TransferView {
        let done = self.base.load(std::sync::atomic::Ordering::Relaxed)
            + self.current.load(std::sync::atomic::Ordering::Relaxed);
        TransferView {
            id: self.id,
            direction: self.direction,
            peer_name: self.peer_name.clone(),
            name: self.name.clone(),
            items: self.items,
            files: self.files,
            total_bytes: self.total,
            done_bytes: done.min(self.total),
            state: self.state,
            error: self.error.clone(),
            started_unix_ms: self.started_ms,
            finished_unix_ms: self.finished_ms,
            path: self.path.as_ref().map(|p| p.display().to_string()),
        }
    }
}

#[derive(Default)]
pub struct Transfers {
    list: Vec<Transfer>,
    next: u64,
}

impl Transfers {
    pub fn next_id(&mut self) -> u64 {
        self.next += 1;
        self.next
    }

    pub fn add(&mut self, transfer: Transfer) {
        self.list.push(transfer);
        while self.list.len() > MAX_TRANSFERS {
            match self.list.iter().position(Transfer::finished) {
                Some(i) => {
                    self.list.remove(i);
                }
                None => break,
            }
        }
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut Transfer> {
        self.list.iter_mut().find(|t| t.id == id)
    }

    pub fn get(&self, id: u64) -> Option<&Transfer> {
        self.list.iter().find(|t| t.id == id)
    }

    pub fn by_offer(&mut self, offer_id: u64, direction: TransferDirection) -> Option<&mut Transfer> {
        self.list
            .iter_mut()
            .find(|t| t.offer_id == offer_id && t.direction == direction)
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Transfer> {
        self.list.iter_mut()
    }

    pub fn active(&self) -> bool {
        self.list.iter().any(|t| !t.finished())
    }

    pub fn clear_finished(&mut self) {
        self.list.retain(|t| !t.finished());
    }

    pub fn views(&self) -> Vec<TransferView> {
        self.list.iter().rev().map(Transfer::view).collect()
    }
}
