use std::sync::{Arc, Weak};
use std::time::Duration;

use tautra_clipboard::{
    dib_to_png, ClipboardHandler, LocalContent, LocalImage, RemoteContent, RemoteFile, RemoteFiles,
    RemoteItem,
};
use tautra_common::config::ShareConfig;
use tautra_common::time::unix_ms;
use tautra_ipc::api::{app_error, ClipActivity, ClipKind, NoticeLevel};
use tautra_proto::frame;
use tautra_proto::share::{
    ClipFormat, ClipItem, ClipOffer, ClipPayload, FileOffer, ShareMessage, INLINE_LIMIT,
};
use tautra_transfer::{BlockingReader, Manifest};
use tokio::runtime::Handle;

use super::net;
use crate::app::Agent;

const FETCH_TIMEOUT: Duration = Duration::from_secs(30);

pub struct ClipHandler {
    agent: Weak<Agent>,
    runtime: Handle,
}

impl ClipHandler {
    pub fn new(agent: &Arc<Agent>, runtime: Handle) -> Self {
        Self {
            agent: Arc::downgrade(agent),
            runtime,
        }
    }

    fn agent(&self) -> Result<Arc<Agent>, String> {
        self.agent
            .upgrade()
            .ok_or_else(|| "agent Tautra sedang berhenti".to_string())
    }
}

struct StreamReader(BlockingReader);

impl RemoteFile for StreamReader {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, String> {
        self.0.read(buf)
    }
}

impl ClipboardHandler for ClipHandler {
    fn local_changed(&self, content: LocalContent) {
        if let Ok(agent) = self.agent() {
            self.runtime.spawn(publish(agent, content));
        }
    }

    fn fetch(&self, offer_id: u64, format: ClipFormat) -> Result<Vec<u8>, String> {
        let agent = self.agent()?;
        if agent.share.connection().is_none() {
            return Err("perangkat lain tidak tersambung".into());
        }
        let conns = agent.share.connections();
        let max = agent.config().share.max_clipboard_bytes();
        self.runtime.block_on(async {
            tokio::time::timeout(FETCH_TIMEOUT, net::fetch_clip(conns, offer_id, format, max))
                .await
                .map_err(|_| {
                    format!(
                        "perangkat lain tidak mengirim isi clipboard dalam {} detik",
                        FETCH_TIMEOUT.as_secs()
                    )
                })?
        })
    }

    fn open_file(
        &self,
        offer_id: u64,
        index: u32,
        size: u64,
    ) -> Result<Box<dyn RemoteFile>, String> {
        let agent = self.agent()?;
        if agent.share.connection().is_none() {
            return Err("perangkat lain tidak tersambung".into());
        }
        let reader = tautra_transfer::stream_file(
            &self.runtime,
            agent.share.connections(),
            offer_id,
            index,
            size,
        );
        Ok(Box::new(StreamReader(reader)))
    }

    fn error(&self, message: String) {
        if let Ok(agent) = self.agent() {
            agent.notice(NoticeLevel::Warning, app_error::CLIPBOARD_FAILED, message);
        }
    }
}

fn kind(format: ClipFormat) -> ClipKind {
    match format {
        ClipFormat::Text => ClipKind::Text,
        ClipFormat::Html => ClipKind::Html,
        ClipFormat::Rtf => ClipKind::Rtf,
        ClipFormat::Png => ClipKind::Image,
    }
}

fn allowed(format: ClipFormat, config: &ShareConfig) -> bool {
    match format {
        ClipFormat::Text => config.text,
        ClipFormat::Html => config.html,
        ClipFormat::Rtf => config.rtf,
        ClipFormat::Png => config.image,
    }
}

fn activity(formats: impl Iterator<Item = (ClipFormat, u64)>, files: Option<&FileOffer>) -> ClipActivity {
    let mut kinds = Vec::new();
    let mut bytes = 0;
    for (format, size) in formats {
        kinds.push(kind(format));
        bytes += size;
    }
    let mut file_count = 0;
    if let Some(files) = files {
        kinds.push(ClipKind::Files);
        bytes += files.total_bytes();
        file_count = files.file_count() as u32;
    }
    ClipActivity {
        at_unix_ms: unix_ms(),
        kinds,
        bytes,
        files: file_count,
    }
}

async fn encode_image(image: LocalImage) -> Result<Vec<u8>, String> {
    match image {
        LocalImage::Png(png) => Ok(png),
        LocalImage::Dib(dib) => tokio::task::spawn_blocking(move || dib_to_png(&dib))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string()),
    }
}

async fn manifest(paths: Vec<std::path::PathBuf>) -> Result<Manifest, String> {
    tokio::task::spawn_blocking(move || tautra_transfer::build_manifest(&paths))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

fn build_offer(id: u64, data: &[(ClipFormat, Arc<Vec<u8>>)], files: Option<FileOffer>) -> ClipOffer {
    let mut inline_budget = INLINE_LIMIT;
    let items = data
        .iter()
        .map(|(format, bytes)| {
            let size = bytes.len() as u64;
            let payload = if size <= inline_budget {
                inline_budget -= size;
                ClipPayload::Inline(bytes.to_vec())
            } else {
                ClipPayload::Lazy { size }
            };
            ClipItem {
                format: *format,
                payload,
            }
        })
        .collect();
    ClipOffer { id, items, files }
}

pub async fn publish(agent: Arc<Agent>, content: LocalContent) {
    let config = agent.config().share;
    if !config.clipboard || agent.share.connection().is_none() {
        return;
    }
    let max = config.max_clipboard_bytes();
    let mut data: Vec<(ClipFormat, Vec<u8>)> = Vec::new();
    if let Some(text) = content.text.filter(|_| config.text) {
        data.push((ClipFormat::Text, text.into_bytes()));
    }
    if let Some(html) = content.html.filter(|_| config.html) {
        data.push((ClipFormat::Html, html));
    }
    if let Some(rtf) = content.rtf.filter(|_| config.rtf) {
        data.push((ClipFormat::Rtf, rtf));
    }
    if let Some(image) = content.image.filter(|_| config.image) {
        match encode_image(image).await {
            Ok(png) => data.push((ClipFormat::Png, png)),
            Err(e) => {
                tracing::warn!("gambar di clipboard tidak bisa dikirim: {e}");
                agent.notice(
                    NoticeLevel::Warning,
                    app_error::CLIPBOARD_FAILED,
                    format!("Gambar di clipboard tidak bisa dikirim: {e}"),
                );
            }
        }
    }
    let before = data.len();
    data.retain(|(_, bytes)| bytes.len() as u64 <= max);
    if data.len() < before {
        tracing::info!("sebagian isi clipboard melebihi batas {} MB dan tidak dikirim", config.max_clipboard_mb);
    }
    let files = if config.files && !content.files.is_empty() {
        match manifest(content.files).await {
            Ok(manifest) => Some(manifest),
            Err(e) => {
                tracing::warn!("file di clipboard tidak bisa dibagikan: {e}");
                agent.notice(
                    NoticeLevel::Warning,
                    app_error::CLIPBOARD_FAILED,
                    format!("File yang disalin tidak bisa dibagikan: {e}"),
                );
                None
            }
        }
    } else {
        None
    };
    if data.is_empty() && files.is_none() {
        return;
    }
    let data: Vec<(ClipFormat, Arc<Vec<u8>>)> =
        data.into_iter().map(|(f, d)| (f, Arc::new(d))).collect();
    let entries = files.as_ref().map(|m| m.entries.clone());
    let mut dropped_files = false;
    let mut sent_activity = None;
    let sent = agent.share.offer_clipboard(data.clone(), files, |id| {
        let file_offer = entries.map(|entries| FileOffer { id, entries });
        let mut offer = build_offer(id, &data, file_offer);
        if offer.files.is_some()
            && frame::encode(&ShareMessage::Clipboard(offer.clone())).is_err()
        {
            offer.files = None;
            dropped_files = true;
        }
        sent_activity = Some(activity(
            offer.items.iter().map(|i| (i.format, i.payload.size())),
            offer.files.as_ref(),
        ));
        ShareMessage::Clipboard(offer)
    });
    if dropped_files {
        agent.notice(
            NoticeLevel::Warning,
            app_error::CLIPBOARD_FAILED,
            "Daftar file yang disalin terlalu panjang untuk dikirim lewat clipboard; seret file ke perangkat lain sebagai gantinya",
        );
    }
    if sent {
        if let Some(a) = sent_activity {
            tracing::debug!(kinds = ?a.kinds, bytes = a.bytes, "clipboard dikirim");
            agent.share.record_sent(a);
        }
        agent.touch();
    }
}

pub fn receive(agent: &Arc<Agent>, offer: ClipOffer) {
    let config = agent.config().share;
    if !config.clipboard {
        return;
    }
    let max = config.max_clipboard_bytes();
    let items: Vec<RemoteItem> = offer
        .items
        .into_iter()
        .filter(|i| allowed(i.format, &config) && i.payload.size() <= max)
        .map(|i| {
            let size = i.payload.size();
            RemoteItem {
                format: i.format,
                size,
                inline: match i.payload {
                    ClipPayload::Inline(data) => Some(data),
                    ClipPayload::Lazy { .. } => None,
                },
            }
        })
        .collect();
    let files = offer.files.filter(|f| config.files && !f.entries.is_empty());
    if items.is_empty() && files.is_none() {
        return;
    }
    let received = activity(items.iter().map(|i| (i.format, i.size)), files.as_ref());
    let content = RemoteContent {
        offer_id: offer.id,
        items,
        files: files.map(|f| RemoteFiles {
            offer_id: f.id,
            entries: f.entries,
        }),
    };
    if agent.share.set_remote_clipboard(content) {
        tracing::debug!(kinds = ?received.kinds, bytes = received.bytes, "clipboard diterima");
        agent.share.record_received(received);
        agent.touch();
    }
}
