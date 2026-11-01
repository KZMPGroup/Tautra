use std::collections::HashMap;
use std::ffi::c_void;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, PipeMode, ServerOptions,
};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_util::codec::{FramedRead, FramedWrite, LinesCodec};
use tokio_util::sync::CancellationToken;
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, LocalFree, ERROR_PIPE_BUSY, HANDLE, HLOCAL};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{
    GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY,
    TOKEN_USER,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use crate::jsonrpc::{self, Incoming, Notification, Request, Response, RpcError};
use crate::MAX_MESSAGE_LEN;

struct PipeSecurity {
    descriptor: PSECURITY_DESCRIPTOR,
    attributes: SECURITY_ATTRIBUTES,
}

unsafe impl Send for PipeSecurity {}
unsafe impl Sync for PipeSecurity {}

impl PipeSecurity {
    fn current_user_only() -> io::Result<Box<Self>> {
        let sid = current_user_sid()?;
        let sddl = format!("D:P(A;;GA;;;SY)(A;;GA;;;{sid})");
        let wide: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(wide.as_ptr()),
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )
            .map_err(win_to_io)?;
        }
        let mut security = Box::new(Self {
            descriptor,
            attributes: SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: std::ptr::null_mut(),
                bInheritHandle: false.into(),
            },
        });
        security.attributes.lpSecurityDescriptor = security.descriptor.0;
        Ok(security)
    }

    fn as_raw(&self) -> *mut c_void {
        &self.attributes as *const SECURITY_ATTRIBUTES as *mut c_void
    }
}

impl Drop for PipeSecurity {
    fn drop(&mut self) {
        if !self.descriptor.0.is_null() {
            unsafe {
                LocalFree(Some(HLOCAL(self.descriptor.0)));
            }
        }
    }
}

fn current_user_sid() -> io::Result<String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).map_err(win_to_io)?;
        let result = (|| {
            let mut len = 0u32;
            let _ = GetTokenInformation(token, TokenUser, None, 0, &mut len);
            if len == 0 {
                return Err(io::Error::other(
                    "GetTokenInformation tidak mengembalikan ukuran",
                ));
            }
            let mut buf = vec![0u64; (len as usize).div_ceil(8)];
            GetTokenInformation(
                token,
                TokenUser,
                Some(buf.as_mut_ptr().cast()),
                len,
                &mut len,
            )
            .map_err(win_to_io)?;
            let user = &*(buf.as_ptr() as *const TOKEN_USER);
            let mut sid_str = PWSTR::null();
            ConvertSidToStringSidW(user.User.Sid, &mut sid_str).map_err(win_to_io)?;
            let sid = sid_str.to_string().map_err(io::Error::other);
            LocalFree(Some(HLOCAL(sid_str.0.cast())));
            sid
        })();
        let _ = CloseHandle(token);
        result
    }
}

fn win_to_io(err: windows::core::Error) -> io::Error {
    io::Error::from_raw_os_error(err.code().0 & 0xFFFF)
}

pub type HandlerFuture = Pin<Box<dyn Future<Output = Result<Value, RpcError>> + Send>>;

pub trait RpcHandler: Send + Sync + 'static {
    fn handle(&self, method: String, params: Value) -> HandlerFuture;
}

fn create_instance(
    name: &str,
    security: &PipeSecurity,
    first: bool,
) -> io::Result<NamedPipeServer> {
    let mut options = ServerOptions::new();
    options
        .first_pipe_instance(first)
        .reject_remote_clients(true)
        .pipe_mode(PipeMode::Byte);
    unsafe { options.create_with_security_attributes_raw(name, security.as_raw()) }
}

pub async fn serve(
    name: &str,
    handler: Arc<dyn RpcHandler>,
    notifications: broadcast::Sender<Arc<str>>,
    cancel: CancellationToken,
) -> io::Result<()> {
    let security = PipeSecurity::current_user_only()?;
    let mut server = create_instance(name, &security, true)?;
    tracing::info!(pipe = name, "server IPC siap");

    loop {
        tokio::select! {
            res = server.connect() => {
                if let Err(e) = res {
                    tracing::warn!("koneksi klien IPC gagal: {e}");
                    server = create_instance(name, &security, false)?;
                    continue;
                }
            }
            _ = cancel.cancelled() => return Ok(()),
        }
        let next = create_instance(name, &security, false)?;
        let client = std::mem::replace(&mut server, next);
        tokio::spawn(handle_client(
            client,
            handler.clone(),
            notifications.subscribe(),
            cancel.child_token(),
        ));
    }
}

async fn handle_client(
    pipe: NamedPipeServer,
    handler: Arc<dyn RpcHandler>,
    mut notifications: broadcast::Receiver<Arc<str>>,
    cancel: CancellationToken,
) {
    tracing::debug!("klien IPC tersambung");
    let (rd, wr) = tokio::io::split(pipe);
    let mut lines = FramedRead::new(rd, LinesCodec::new_with_max_length(MAX_MESSAGE_LEN));
    let mut sink = FramedWrite::new(wr, LinesCodec::new_with_max_length(MAX_MESSAGE_LEN));
    let (resp_tx, mut resp_rx) = mpsc::channel::<String>(64);

    loop {
        let outgoing: String = tokio::select! {
            line = lines.next() => match line {
                Some(Ok(line)) => {
                    if let Some(reply) = dispatch(&line, &handler, &resp_tx) {
                        reply
                    } else {
                        continue;
                    }
                }
                Some(Err(e)) => {
                    tracing::warn!("pesan IPC tidak valid, klien diputus: {e}");
                    break;
                }
                None => break,
            },
            Some(reply) = resp_rx.recv() => reply,
            note = notifications.recv() => match note {
                Ok(text) => text.to_string(),
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            },
            _ = cancel.cancelled() => break,
        };
        if let Err(e) = sink.send(outgoing).await {
            tracing::debug!("gagal menulis ke klien IPC: {e}");
            break;
        }
    }
    tracing::debug!("klien IPC terputus");
}

fn dispatch(
    line: &str,
    handler: &Arc<dyn RpcHandler>,
    resp_tx: &mpsc::Sender<String>,
) -> Option<String> {
    match jsonrpc::parse(line) {
        Ok(Incoming::Request(Request {
            id, method, params, ..
        })) => {
            let handler = handler.clone();
            let resp_tx = resp_tx.clone();
            tokio::spawn(async move {
                let result = handler
                    .handle(method.clone(), params.unwrap_or(Value::Null))
                    .await;
                let response = match result {
                    Ok(value) => Response::ok(id, value),
                    Err(err) => {
                        tracing::debug!(method, "request IPC gagal: {err}");
                        Response::err(Some(id), err)
                    }
                };
                if let Ok(text) = serde_json::to_string(&response) {
                    let _ = resp_tx.send(text).await;
                }
            });
            None
        }
        Ok(_) => None,
        Err(err) => serde_json::to_string(&Response::err(None, err)).ok(),
    }
}

pub fn notification_text<T: Serialize>(method: &str, params: &T) -> Option<Arc<str>> {
    let params = serde_json::to_value(params).ok()?;
    serde_json::to_string(&Notification::new(method, params))
        .ok()
        .map(Arc::from)
}

#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error("tidak tersambung ke agent")]
    Disconnected,
    #[error("agent tidak merespons dalam {0:?}")]
    Timeout(Duration),
    #[error(transparent)]
    Rpc(#[from] RpcError),
    #[error("respons agent tidak valid: {0}")]
    Decode(serde_json::Error),
    #[error("parameter tidak bisa diserialisasi: {0}")]
    Encode(serde_json::Error),
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Response>>>>;

#[derive(Clone)]
pub struct IpcClient {
    next_id: Arc<AtomicU64>,
    pending: Pending,
    out_tx: mpsc::Sender<String>,
    closed: CancellationToken,
}

impl IpcClient {
    pub async fn connect(name: &str) -> io::Result<(Self, mpsc::Receiver<Notification>)> {
        let pipe = open_with_retry(name).await?;
        let (rd, wr) = tokio::io::split(pipe);
        let (out_tx, out_rx) = mpsc::channel::<String>(64);
        let (note_tx, note_rx) = mpsc::channel::<Notification>(64);
        let pending: Pending = Arc::default();
        let closed = CancellationToken::new();

        tokio::spawn(client_io(
            rd,
            wr,
            out_rx,
            note_tx,
            pending.clone(),
            closed.clone(),
        ));

        Ok((
            Self {
                next_id: Arc::new(AtomicU64::new(1)),
                pending,
                out_tx,
                closed,
            },
            note_rx,
        ))
    }

    pub fn is_closed(&self) -> bool {
        self.closed.is_cancelled()
    }

    pub async fn closed(&self) {
        self.closed.cancelled().await
    }

    pub async fn call<P: Serialize, R: DeserializeOwned>(
        &self,
        method: &str,
        params: P,
        timeout: Duration,
    ) -> Result<R, IpcError> {
        let value = self
            .call_value(
                method,
                serde_json::to_value(params).map_err(IpcError::Encode)?,
                timeout,
            )
            .await?;
        serde_json::from_value(value).map_err(IpcError::Decode)
    }

    pub async fn call_value(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, IpcError> {
        if self.is_closed() {
            return Err(IpcError::Disconnected);
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request = Request {
            jsonrpc: jsonrpc::VERSION.into(),
            id,
            method: method.to_string(),
            params: (!params.is_null()).then_some(params),
        };
        let text = serde_json::to_string(&request).map_err(IpcError::Encode)?;
        let (tx, rx) = oneshot::channel();
        self.pending.lock().expect("pending lock").insert(id, tx);

        if self.out_tx.send(text).await.is_err() {
            self.pending.lock().expect("pending lock").remove(&id);
            return Err(IpcError::Disconnected);
        }

        let response = match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(response)) => response,
            Ok(Err(_)) => return Err(IpcError::Disconnected),
            Err(_) => {
                self.pending.lock().expect("pending lock").remove(&id);
                return Err(IpcError::Timeout(timeout));
            }
        };
        match (response.result, response.error) {
            (_, Some(err)) => Err(IpcError::Rpc(err)),
            (Some(value), None) => Ok(value),
            (None, None) => Ok(Value::Null),
        }
    }
}

async fn open_with_retry(name: &str) -> io::Result<NamedPipeClient> {
    const MAX_BUSY_RETRIES: u32 = 40;
    let mut attempts = 0;
    loop {
        match ClientOptions::new().open(name) {
            Ok(client) => return Ok(client),
            Err(e)
                if e.raw_os_error() == Some(ERROR_PIPE_BUSY.0 as i32)
                    && attempts < MAX_BUSY_RETRIES =>
            {
                attempts += 1;
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Err(e) => return Err(e),
        }
    }
}

async fn client_io(
    rd: tokio::io::ReadHalf<NamedPipeClient>,
    wr: tokio::io::WriteHalf<NamedPipeClient>,
    mut out_rx: mpsc::Receiver<String>,
    note_tx: mpsc::Sender<Notification>,
    pending: Pending,
    closed: CancellationToken,
) {
    let mut lines = FramedRead::new(rd, LinesCodec::new_with_max_length(MAX_MESSAGE_LEN));
    let mut sink = FramedWrite::new(wr, LinesCodec::new_with_max_length(MAX_MESSAGE_LEN));

    loop {
        tokio::select! {
            line = lines.next() => match line {
                Some(Ok(line)) => match jsonrpc::parse(&line) {
                    Ok(Incoming::Response(resp)) => {
                        if let Some(id) = resp.id {
                            if let Some(tx) = pending.lock().expect("pending lock").remove(&id) {
                                let _ = tx.send(resp);
                            }
                        } else if let Some(err) = resp.error {
                            tracing::warn!("agent menolak pesan: {err}");
                        }
                    }
                    Ok(Incoming::Notification(note)) => {
                        let _ = note_tx.try_send(note);
                    }
                    Ok(Incoming::Request(_)) => {}
                    Err(e) => tracing::warn!("pesan agent tidak valid: {e}"),
                },
                Some(Err(e)) => {
                    tracing::warn!("koneksi IPC rusak: {e}");
                    break;
                }
                None => break,
            },
            out = out_rx.recv() => match out {
                Some(text) => {
                    if let Err(e) = sink.send(text).await {
                        tracing::warn!("gagal mengirim ke agent: {e}");
                        break;
                    }
                }
                None => break,
            },
        }
    }

    closed.cancel();
    pending.lock().expect("pending lock").clear();
}
