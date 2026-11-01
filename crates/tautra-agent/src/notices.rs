use std::collections::VecDeque;

use tautra_common::time::unix_ms;
use tautra_ipc::api::{Notice, NoticeLevel};

const MAX_NOTICES: usize = 20;

#[derive(Default)]
pub struct NoticeBoard {
    next_id: u64,
    items: VecDeque<Notice>,
}

impl NoticeBoard {
    pub fn push(&mut self, level: NoticeLevel, code: &str, message: impl Into<String>) {
        let message = message.into();
        if let Some(existing) = self
            .items
            .iter_mut()
            .find(|n| n.code == code && n.message == message)
        {
            existing.at_unix_ms = unix_ms();
            return;
        }
        self.next_id += 1;
        self.items.push_back(Notice {
            id: self.next_id,
            level,
            code: code.to_string(),
            message,
            at_unix_ms: unix_ms(),
        });
        while self.items.len() > MAX_NOTICES {
            self.items.pop_front();
        }
    }

    pub fn dismiss(&mut self, id: u64) -> bool {
        let before = self.items.len();
        self.items.retain(|n| n.id != id);
        before != self.items.len()
    }

    pub fn list(&self) -> Vec<Notice> {
        self.items.iter().cloned().collect()
    }
}
