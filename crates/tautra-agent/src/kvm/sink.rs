use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use bytes::BytesMut;
use tautra_input::{DragCheck, HookStatus, InputSink};
use tautra_proto::kvm::{encode_mouse_move, KvmMessage, MOUSE_MOVE_LEN};
use tokio::sync::{mpsc, watch};

use crate::app::Agent;

const DATAGRAM_ARENA: usize = 16 * 1024;

pub struct KvmSink {
    agent: Arc<Agent>,
    conn: watch::Receiver<quinn::Connection>,
    out: mpsc::Sender<KvmMessage>,
    seq: AtomicU32,
    arena: Mutex<BytesMut>,
    drag_skipped: AtomicBool,
}

impl KvmSink {
    pub fn new(
        agent: Arc<Agent>,
        conn: watch::Receiver<quinn::Connection>,
        out: mpsc::Sender<KvmMessage>,
    ) -> Self {
        Self {
            agent,
            conn,
            out,
            seq: AtomicU32::new(0),
            arena: Mutex::new(BytesMut::with_capacity(DATAGRAM_ARENA)),
            drag_skipped: AtomicBool::new(false),
        }
    }
}

impl InputSink for KvmSink {
    fn mouse_move(&self, x: i32, y: i32) {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
        let packet = {
            let Ok(mut arena) = self.arena.lock() else {
                return;
            };
            if arena.capacity() < MOUSE_MOVE_LEN {
                arena.reserve(DATAGRAM_ARENA);
            }
            arena.extend_from_slice(&encode_mouse_move(seq, x, y));
            arena.split().freeze()
        };
        let conn = self.conn.borrow().clone();
        if let Err(e) = conn.send_datagram(packet) {
            tracing::debug!("datagram gerakan mouse gagal dikirim: {e}");
        }
    }

    fn message(&self, msg: KvmMessage) {
        if let Err(e) = self.out.try_send(msg) {
            tracing::warn!("antrean input ke perangkat lain penuh atau tertutup: {e}");
        }
    }

    fn status(&self, status: HookStatus) {
        self.agent.kvm.hook_status(status);
        self.agent.touch();
    }

    fn drag_probe(&self, x: i32, y: i32) {
        let enabled = self.agent.config().share.drag_drop;
        let probing = self.agent.share.drag_probe(enabled, x, y);
        self.drag_skipped.store(!probing, Ordering::Release);
    }

    fn drag_state(&self) -> DragCheck {
        if self.drag_skipped.load(Ordering::Acquire) {
            DragCheck::NoFiles
        } else {
            self.agent.share.drag_check()
        }
    }

    fn drag_drop(&self) {
        self.agent.share.drag_drop(true);
    }

    fn drag_cancel(&self) {
        self.agent.share.drag_cancel();
    }

    fn drag_return(&self) -> bool {
        match self.agent.share.take_incoming_drag() {
            Some(offer_id) => {
                self.message(KvmMessage::DragReturn { offer_id });
                true
            }
            None => false,
        }
    }
}
