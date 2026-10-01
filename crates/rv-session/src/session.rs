use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine;
use openssl::hash::{Hasher, MessageDigest};
use rv_core::{ConnectRequest, EncryptionMode};
use serde_json::Value;
use tokio::net::TcpStream;
use tokio::time::timeout;
use uuid::Uuid;
use vnc::tight::{TightFileCommand, TightFileEvent};
use vnc::{PixelFormat, VncConnector, VncEvent, X11Event};

use crate::SessionError;
use crate::compositor::{Apply, Framebuffer};
use crate::encodings::encodings_for;
use crate::file_transfer::{
    self, FileCommand, FileTransferClient, FileTransferSnapshot, PHOTO_PAGE_SIZE, PhotoAlbum,
    PhotoEntry, TransferDirection, TransferRecord, TransferStatus,
};
use crate::vencrypt;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Shortest interval between incremental update requests once the previous
/// one has been answered.
const REFRESH_EVERY: Duration = Duration::from_millis(16);
/// Longest interval without a request. Servers hold an incremental request
/// until the screen changes, so this only matters as a keepalive; it also
/// nudges vnc-rs's network task, which stops reading while its decoder is
/// backlogged until the next outgoing message.
const REFRESH_KEEPALIVE: Duration = Duration::from_millis(250);
/// How long the loop idles waiting for input before polling decoded frames again.
const POLL_EVERY: Duration = Duration::from_millis(4);
/// Decoded frame events applied per scheduling slot. A server streaming
/// updates must not keep the loop busy so long that a queued key-up waits;
/// the remote side would auto-repeat the key in the meantime.
const MAX_EVENTS_PER_SLOT: usize = 64;

#[derive(Debug, Clone)]
pub enum SessionEvent {
    Status(String),
    Connected { width: u16, height: u16 },
    FrameReady { generation: u64 },
    Clipboard(String),
    Apps(crate::AppEvent),
    Bell,
    Error(String),
    Disconnected,
}

#[derive(Debug, Clone)]
pub enum SessionCommand {
    Input(X11Event),
    File(FileCommand),
    App(crate::AppCommand),
    Close,
}

pub struct SessionHandle {
    pub framebuffer: Arc<Mutex<Framebuffer>>,
    cmd_tx: tokio::sync::mpsc::UnboundedSender<SessionCommand>,
    event_rx: Mutex<mpsc::Receiver<SessionEvent>>,
    file_state: Arc<Mutex<FileTransferSnapshot>>,
    thread: Option<thread::JoinHandle<()>>,
}

/// In-memory peer for UI tests; no socket or worker thread is created.
#[cfg(feature = "test-support")]
pub struct SessionTestPeer {
    pub commands: tokio::sync::mpsc::UnboundedReceiver<SessionCommand>,
    pub events: mpsc::Sender<SessionEvent>,
}

impl SessionHandle {
    #[cfg(feature = "test-support")]
    pub fn test_pair() -> (Self, SessionTestPeer) {
        let (cmd_tx, commands) = tokio::sync::mpsc::unbounded_channel();
        let (events, event_rx) = mpsc::channel();
        (
            Self {
                framebuffer: Arc::new(Mutex::new(Framebuffer::default())),
                cmd_tx,
                event_rx: Mutex::new(event_rx),
                file_state: Arc::new(Mutex::new(FileTransferSnapshot::default())),
                thread: None,
            },
            SessionTestPeer { commands, events },
        )
    }

    pub fn spawn(request: ConnectRequest) -> Self {
        let framebuffer = Arc::new(Mutex::new(Framebuffer::default()));
        let (cmd_tx, cmd_rx) = tokio::sync::mpsc::unbounded_channel();
        let (event_tx, event_rx) = mpsc::channel();
        let fb = framebuffer.clone();
        let file_state = Arc::new(Mutex::new(FileTransferSnapshot::default()));
        let worker_file_state = file_state.clone();
        let thread = thread::Builder::new()
            .name("rv-vnc".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .expect("tokio runtime");
                rt.block_on(run(request, fb, cmd_rx, event_tx, worker_file_state));
            })
            .expect("spawn vnc thread");
        Self {
            framebuffer,
            cmd_tx,
            event_rx: Mutex::new(event_rx),
            file_state,
            thread: Some(thread),
        }
    }

    pub fn send(&self, cmd: SessionCommand) {
        let _ = self.cmd_tx.send(cmd);
    }

    pub fn file_client(&self) -> FileTransferClient {
        FileTransferClient {
            state: self.file_state.clone(),
            commands: self.cmd_tx.clone(),
        }
    }

    pub fn try_recv(&self) -> Option<SessionEvent> {
        self.event_rx.lock().ok()?.try_recv().ok()
    }

    /// Events since the last call.
    ///
    /// Frame notifications are collapsed to the newest generation (see
    /// [`coalesce_frames`]): pixels live in `framebuffer`, so one notification
    /// per drain is all a renderer needs, no matter how many rectangles the
    /// server sent.
    pub fn drain(&self) -> Vec<SessionEvent> {
        let mut out = Vec::new();
        while let Some(e) = self.try_recv() {
            out.push(e);
        }
        coalesce_frames(&mut out);
        out
    }

    pub fn app(&self, command: crate::AppCommand) {
        self.send(SessionCommand::App(command));
    }

    pub fn pointer(&self, x: u16, y: u16, buttons: u8) {
        self.send(SessionCommand::Input(X11Event::PointerEvent(
            (x, y, buttons).into(),
        )));
    }

    pub fn key(&self, keysym: u32, down: bool) {
        self.send(SessionCommand::Input(X11Event::KeyEvent(
            (keysym, down).into(),
        )));
    }

    pub fn copy_text(&self, text: String) {
        self.send(SessionCommand::Input(X11Event::CopyText(text)));
    }

    pub fn close(&self) {
        self.send(SessionCommand::Close);
    }
}

/// Collapse every `FrameReady` in `events` into a single one carrying the
/// newest generation, at the position of the last frame event. Every other
/// event keeps its relative order.
///
/// Servers send one rectangle per event, and a busy desktop yields dozens per
/// frame. Rebuilding the on-screen image once per rectangle stalls the UI
/// thread badly enough that queued key-ups are delivered late and the remote
/// auto-repeats the key.
pub fn coalesce_frames(events: &mut Vec<SessionEvent>) {
    let mut newest: Option<u64> = None;
    let mut last = None;
    for (i, e) in events.iter().enumerate() {
        if let SessionEvent::FrameReady { generation } = e {
            newest = Some(newest.map_or(*generation, |n| n.max(*generation)));
            last = Some(i);
        }
    }
    let (Some(generation), Some(last)) = (newest, last) else {
        return;
    };
    let mut index = 0;
    events.retain(|e| {
        let keep = !matches!(e, SessionEvent::FrameReady { .. }) || index == last;
        index += 1;
        keep
    });
    if let Some(SessionEvent::FrameReady { generation: g }) = events
        .iter_mut()
        .find(|e| matches!(e, SessionEvent::FrameReady { .. }))
    {
        *g = generation;
    }
}

impl Drop for SessionHandle {
    fn drop(&mut self) {
        // Ask the session to stop but never block the caller: this runs on the
        // UI thread when a window closes, and the worker may be mid-handshake.
        // The worker notices the closed command channel and exits on its own.
        self.close();
        drop(self.thread.take());
    }
}

async fn run(
    request: ConnectRequest,
    fb: Arc<Mutex<Framebuffer>>,
    mut cmd_rx: tokio::sync::mpsc::UnboundedReceiver<SessionCommand>,
    event_tx: mpsc::Sender<SessionEvent>,
    file_state: Arc<Mutex<FileTransferSnapshot>>,
) {
    let send = |e: SessionEvent| {
        let _ = event_tx.send(e);
    };

    send(SessionEvent::Status(format!(
        "Connecting to {}:{}…",
        request.host, request.port
    )));

    match connect_and_loop(request, fb, &mut cmd_rx, &send, file_state.clone()).await {
        Ok(()) => {
            file_transfer::update(&file_state, |state| {
                state.caps = None;
                state.listing = false;
                state.error = Some("Connection closed".into());
            });
            send(SessionEvent::Disconnected);
        }
        Err(e) => {
            file_transfer::update(&file_state, |state| {
                state.caps = None;
                state.listing = false;
                state.error = Some(e.to_string());
            });
            send(SessionEvent::Error(e.to_string()));
            send(SessionEvent::Disconnected);
        }
    }
}

async fn connect_and_loop(
    request: ConnectRequest,
    fb: Arc<Mutex<Framebuffer>>,
    cmd_rx: &mut tokio::sync::mpsc::UnboundedReceiver<SessionCommand>,
    send: &impl Fn(SessionEvent),
    file_state: Arc<Mutex<FileTransferSnapshot>>,
) -> Result<(), SessionError> {
    // A close request (window closed, handle dropped) must interrupt the
    // handshake too, not just the running session; otherwise a server that
    // never answers keeps the worker alive for the whole connect timeout.
    let client = tokio::select! {
        result = connect(&request, send, &file_state) => result?,
        _ = wait_for_close(cmd_rx) => return Ok(()),
    };
    session_loop(client, fb, cmd_rx, send, file_state, !request.view_only).await
}

/// Resolves once the UI asks to close (or drops the handle). Input queued
/// before the session exists has nothing to go to and is discarded.
async fn wait_for_close(cmd_rx: &mut tokio::sync::mpsc::UnboundedReceiver<SessionCommand>) {
    loop {
        match cmd_rx.recv().await {
            Some(SessionCommand::Close) | None => return,
            Some(SessionCommand::Input(_)) => {}
            Some(SessionCommand::File(_)) | Some(SessionCommand::App(_)) => {}
        }
    }
}

async fn dial(addr: &str) -> Result<TcpStream, SessionError> {
    let tcp = timeout(CONNECT_TIMEOUT, TcpStream::connect(addr))
        .await
        .map_err(|_| SessionError::Timeout)?
        .map_err(|e| SessionError::msg(format!("cannot connect to {addr}: {e}")))?;
    let _ = tcp.set_nodelay(true);
    Ok(tcp)
}

/// Which transport to use given what the server offers and what the user asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Transport {
    Plain,
    Tight,
    Ard,
    VeNCrypt,
}

fn choose_transport(
    mode: EncryptionMode,
    types: &[u8],
    has_username: bool,
) -> Result<Transport, SessionError> {
    let plain_ok = types
        .iter()
        .any(|t| matches!(*t, vencrypt::SEC_NONE | vencrypt::SEC_VNC_AUTH));
    let ard_ok = types.contains(&crate::ard::SECURITY_TYPE);
    let tight_ok = types.contains(&16);
    let vencrypt_ok = types.contains(&vencrypt::VENCRYPT_SECURITY_TYPE);
    let offered = || {
        types
            .iter()
            .map(|t| vencrypt::security_type_name(*t))
            .collect::<Vec<_>>()
            .join(", ")
    };
    match mode {
        EncryptionMode::Always if vencrypt_ok => Ok(Transport::VeNCrypt),
        EncryptionMode::Always => Err(SessionError::msg(format!(
            "server does not offer VeNCrypt (offers: {}); set Encryption to Let server choose to connect unencrypted",
            offered()
        ))),
        EncryptionMode::PreferOn if vencrypt_ok => Ok(Transport::VeNCrypt),
        _ if ard_ok && has_username => Ok(Transport::Ard),
        _ if tight_ok => Ok(Transport::Tight),
        EncryptionMode::LetServerChoose if vencrypt_ok && !plain_ok => Ok(Transport::VeNCrypt),
        _ if ard_ok && !plain_ok => Ok(Transport::Ard),
        _ if plain_ok => Ok(Transport::Plain),
        EncryptionMode::Off if vencrypt_ok => Err(SessionError::msg(format!(
            "server requires encryption (offers: {}); set Encryption to Let server choose",
            offered()
        ))),
        _ => Err(SessionError::msg(format!(
            "no supported security type (server offers: {}; RV supports None, VncAuth, Tight, ARD, VeNCrypt)",
            offered()
        ))),
    }
}

async fn connect(
    request: &ConnectRequest,
    send: &impl Fn(SessionEvent),
    file_state: &Arc<Mutex<FileTransferSnapshot>>,
) -> Result<vnc::VncClient, SessionError> {
    let addr = format!("{}:{}", request.host, request.port);
    let mut tcp = dial(&addr).await?;
    let types = vencrypt::read_security_types(&mut tcp).await?;
    let transport = choose_transport(
        request.encryption,
        &types,
        request.username.as_ref().is_some_and(|u| !u.is_empty()),
    )?;
    if transport != Transport::Tight {
        file_transfer::update(file_state, |state| state.caps = Some(Default::default()));
    }
    let stream = match transport {
        Transport::Plain => vencrypt::RfbStream::plain(tcp, &types),
        Transport::Tight => {
            send(SessionEvent::Status(
                "Negotiating TightVNC file transfer…".into(),
            ));
            let auth = timeout(
                CONNECT_TIMEOUT,
                vencrypt::tight_handshake(
                    &mut tcp,
                    request
                        .password
                        .as_ref()
                        .is_some_and(|password| !password.is_empty()),
                ),
            )
            .await
            .map_err(|_| SessionError::Timeout)??;
            vencrypt::RfbStream::tight(tcp, auth)
        }
        Transport::Ard => {
            send(SessionEvent::Status(
                "Authenticating with Mac login…".into(),
            ));
            timeout(
                CONNECT_TIMEOUT,
                crate::ard::handshake(
                    &mut tcp,
                    request.username.as_deref(),
                    request.password.as_deref(),
                ),
            )
            .await
            .map_err(|_| SessionError::Timeout)??;
            vencrypt::RfbStream::authenticated(tcp)
        }
        Transport::VeNCrypt => {
            send(SessionEvent::Status("Negotiating VeNCrypt…".into()));
            vencrypt::handshake(tcp, &request.host).await?
        }
    };

    let password = request.password.clone().unwrap_or_default();
    let mut connector = VncConnector::new(stream)
        .set_auth_method(async move { Ok(password) })
        .allow_shared(request.shared)
        .set_pixel_format(PixelFormat::bgra());
    connector = connector.tight_security(transport == Transport::Tight);
    for enc in encodings_for(request.quality, request.clipboard) {
        connector = connector.add_encoding(enc);
    }
    Ok(connector.build()?.try_start().await?.finish()?)
}

async fn session_loop(
    mut client: vnc::VncClient,
    fb: Arc<Mutex<Framebuffer>>,
    cmd_rx: &mut tokio::sync::mpsc::UnboundedReceiver<SessionCommand>,
    send: &impl Fn(SessionEvent),
    file_state: Arc<Mutex<FileTransferSnapshot>>,
    allow_upload: bool,
) -> Result<(), SessionError> {
    use tokio::sync::mpsc::error::TryRecvError;

    send(SessionEvent::Status(
        "Connected, waiting for desktop…".into(),
    ));
    let mut announced = false;
    let mut last_refresh = Instant::now();
    // One incremental request in flight at a time, like other VNC viewers.
    // Firing them unconditionally queues every key event behind a backlog of
    // requests the server has not answered yet.
    let mut refresh_answered = true;
    let mut files = FileRuntime::new(file_state, allow_upload);
    let mut apps = crate::apps::AppRuntime::new(allow_upload);
    let app_send = |event| send(SessionEvent::Apps(event));
    loop {
        // Input first: a key-up queued behind frame decoding turns into
        // auto-repeat on the server, so pixels never take priority over commands.
        loop {
            match cmd_rx.try_recv() {
                Ok(cmd) => {
                    if !apply_command(&mut client, cmd, &mut files, &mut apps, &app_send).await? {
                        return Ok(());
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    let _ = client.close().await;
                    return Ok(());
                }
            }
        }

        let mut applied = 0;
        while applied < MAX_EVENTS_PER_SLOT {
            match client.poll_event().await {
                Ok(Some(ev)) => {
                    if let VncEvent::Device(reply) = ev {
                        apps.reply(reply, &app_send);
                    } else if let VncEvent::TightFile(event) = ev {
                        if let TightFileEvent::Capabilities(caps) = event {
                            files.event(TightFileEvent::Capabilities(caps));
                            if caps.list {
                                files.command(&client, FileCommand::List("/".into())).await;
                            }
                        } else {
                            files.event(event);
                        }
                    } else {
                        handle_event(ev, &fb, send, &mut announced);
                    }
                    applied += 1;
                    refresh_answered = true;
                }
                Ok(None) => break,
                Err(e) => {
                    let _ = client.close().await;
                    return Err(e.into());
                }
            }
        }
        apps.advance(&client, &app_send).await;
        files.advance_upload(&client).await;
        if let Some(command) = files.followup.pop_front() {
            files.command(&client, command).await;
        }
        files.advance_photo(&client).await;
        let since_refresh = last_refresh.elapsed();
        if (refresh_answered && since_refresh >= REFRESH_EVERY)
            || since_refresh >= REFRESH_KEEPALIVE
        {
            last_refresh = Instant::now();
            refresh_answered = false;
            let _ = client.input(X11Event::Refresh).await;
        }
        if applied == MAX_EVENTS_PER_SLOT {
            // Backlog: go straight back to the input check instead of idling.
            tokio::task::yield_now().await;
            continue;
        }

        tokio::select! {
            cmd = cmd_rx.recv() => {
                match cmd {
                    None => {
                        let _ = client.close().await;
                        return Ok(());
                    }
                    Some(cmd) => {
                        if !apply_command(&mut client, cmd, &mut files, &mut apps, &app_send).await? {
                            return Ok(());
                        }
                    }
                }
            }
            _ = tokio::time::sleep(POLL_EVERY) => {}
        }
    }
}

/// Returns `Ok(false)` once the session should end.
async fn apply_command(
    client: &mut vnc::VncClient,
    cmd: SessionCommand,
    files: &mut FileRuntime,
    apps: &mut crate::apps::AppRuntime,
    app_send: &impl Fn(crate::AppEvent),
) -> Result<bool, SessionError> {
    match cmd {
        SessionCommand::Close => {
            let _ = client.close().await;
            Ok(false)
        }
        SessionCommand::Input(ev) => {
            client.input(ev).await?;
            Ok(true)
        }
        SessionCommand::App(command) => {
            apps.command(command, app_send);
            Ok(true)
        }
        SessionCommand::File(command) => {
            files.command(client, command).await;
            Ok(true)
        }
    }
}

fn handle_event(
    ev: VncEvent,
    fb: &Arc<Mutex<Framebuffer>>,
    send: &impl Fn(SessionEvent),
    announced: &mut bool,
) {
    let mut fb = fb.lock().expect("framebuffer lock");
    match fb.apply(ev) {
        Apply::Resized => {
            if !*announced && fb.width > 0 {
                *announced = true;
                send(SessionEvent::Connected {
                    width: fb.width,
                    height: fb.height,
                });
            }
            send(SessionEvent::FrameReady {
                generation: fb.generation,
            });
        }
        Apply::Dirty => send(SessionEvent::FrameReady {
            generation: fb.generation,
        }),
        Apply::Clipboard(text) => send(SessionEvent::Clipboard(text)),
        Apply::Bell => send(SessionEvent::Bell),
        Apply::Error(e) => send(SessionEvent::Error(e)),
        Apply::Ignored => {}
    }
}

enum ActiveFile {
    Upload {
        file: File,
        remote: String,
        modified: u32,
        index: usize,
        hasher: Hasher,
    },
    Download {
        file: File,
        remote: String,
        destination: PathBuf,
        index: usize,
        hasher: Hasher,
    },
}

struct PendingUploadVerification {
    path: String,
    name: String,
    size: u64,
    index: usize,
    expected_sha256: String,
}

struct PendingChecksum {
    id: u32,
    index: usize,
    expected: String,
    destination: Option<PathBuf>,
}

enum PhotoAction {
    List {
        offset: usize,
        album: Option<String>,
    },
    Import {
        restore_path: Option<String>,
    },
    Export {
        destination: PathBuf,
    },
    Delete {
        album: Option<String>,
        count: usize,
    },
}

struct PendingPhoto {
    id: u32,
    op: u8,
    token: Option<String>,
    poll_due: Instant,
    action: PhotoAction,
}

struct PendingPhotoPage {
    offset: usize,
    next_offset: usize,
    album: Option<String>,
    entries: Vec<PhotoEntry>,
    albums: Vec<PhotoAlbum>,
    total: usize,
}

impl PendingPhotoPage {
    fn new(offset: usize, album: Option<String>) -> Self {
        Self {
            offset,
            next_offset: offset,
            album,
            entries: Vec::new(),
            albums: Vec::new(),
            total: 0,
        }
    }

    fn append(
        &mut self,
        offset: usize,
        returned: usize,
        entries: Vec<PhotoEntry>,
        albums: Vec<PhotoAlbum>,
        total: usize,
    ) -> Result<Option<usize>, ()> {
        if offset != self.next_offset {
            return Err(());
        }
        if offset == self.offset {
            self.albums = albums;
        }
        self.total = total;
        let remaining = PHOTO_PAGE_SIZE.saturating_sub(self.entries.len());
        self.entries.extend(entries.into_iter().take(remaining));
        self.next_offset = offset.saturating_add(returned);
        Ok(
            (returned > 0 && self.entries.len() < PHOTO_PAGE_SIZE && self.next_offset < total)
                .then_some(self.next_offset),
        )
    }
}

struct PendingPhotoUpload {
    remote: String,
    expected_sha256: String,
    restore_path: String,
}

struct FileRuntime {
    state: Arc<Mutex<FileTransferSnapshot>>,
    active: Option<ActiveFile>,
    pending_upload: Option<PendingUploadVerification>,
    pending_management: Option<(u32, u8, String)>,
    pending_checksum: Option<PendingChecksum>,
    pending_photo: Option<PendingPhoto>,
    pending_photo_page: Option<PendingPhotoPage>,
    pending_photo_upload: Option<PendingPhotoUpload>,
    photo_upload_source: Option<PathBuf>,
    pending_export_cleanup: Option<String>,
    pending_cleanup_id: Option<u32>,
    followup: VecDeque<FileCommand>,
    next_management_id: u32,
    pending_lists: VecDeque<String>,
    allow_upload: bool,
}

impl FileRuntime {
    fn new(state: Arc<Mutex<FileTransferSnapshot>>, allow_upload: bool) -> Self {
        Self {
            state,
            active: None,
            pending_upload: None,
            pending_management: None,
            pending_checksum: None,
            pending_photo: None,
            pending_photo_page: None,
            pending_photo_upload: None,
            photo_upload_source: None,
            pending_export_cleanup: None,
            pending_cleanup_id: None,
            followup: VecDeque::new(),
            next_management_id: 1,
            pending_lists: VecDeque::new(),
            allow_upload,
        }
    }

    fn fail(&mut self, message: impl Into<String>) {
        let message = message.into();
        file_transfer::update(&self.state, |state| state.error = Some(message));
    }

    fn photo_fail(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.pending_photo = None;
        self.pending_photo_page = None;
        self.photo_upload_source = None;
        if let Some(upload) = self.pending_photo_upload.take() {
            self.followup
                .push_back(FileCommand::PhotoCleanupExport(upload.remote));
            self.followup
                .push_back(FileCommand::List(upload.restore_path));
        }
        file_transfer::update(&self.state, |state| {
            state.photo_busy = false;
            state.photo_error = Some(message);
            state.photo_revision = state.photo_revision.wrapping_add(1);
        });
    }

    fn fail_download(&mut self, remote: &str, message: impl Into<String>) {
        let message = message.into();
        if self.pending_export_cleanup.as_deref() == Some(remote) {
            if let Some(path) = self.pending_export_cleanup.take() {
                self.followup
                    .push_back(FileCommand::PhotoCleanupExport(path));
            }
            self.photo_fail(message.clone());
        }
        self.fail(message);
    }

    async fn start_photo(
        &mut self,
        client: &vnc::VncClient,
        op: u8,
        value: String,
        auxiliary: Option<String>,
        action: PhotoAction,
    ) {
        if !self.state.lock().unwrap().can_photos {
            self.photo_fail("Photos is unavailable on this connection");
            return;
        }
        if self.pending_photo.is_some() {
            file_transfer::update(&self.state, |state| {
                state.photo_error = Some("Wait for the current photo operation to finish".into());
            });
            return;
        }
        if (op == 4 || op == 9) && !self.allow_upload {
            self.photo_fail("Changing photos is disabled in view-only mode");
            return;
        }
        if value.is_empty() || value.len() > 4096 {
            self.photo_fail("Invalid Photos request");
            return;
        }
        let id = self.next_management_id;
        self.next_management_id = self.next_management_id.wrapping_add(1);
        self.pending_photo = Some(PendingPhoto {
            id,
            op,
            token: None,
            poll_due: Instant::now(),
            action,
        });
        file_transfer::update(&self.state, |state| {
            state.photo_busy = true;
            state.photo_error = None;
            if op != 5
                || !state.photo_status.as_deref().is_some_and(|status| {
                    status.starts_with("Imported into Photos") || status.starts_with("Deleted ")
                })
            {
                state.photo_status = Some(
                    match op {
                        4 => "Importing into Photos…",
                        5 => "Loading Photos…",
                        6 => "Preparing original photo…",
                        _ => "Confirm deletion on the iPhone…",
                    }
                    .into(),
                );
            }
        });
        if let Err(error) = client
            .input(X11Event::TightFile(TightFileCommand::Manage {
                op,
                id,
                path: value,
                destination: auxiliary,
            }))
            .await
        {
            self.photo_fail(error.to_string());
        }
    }

    async fn advance_photo(&mut self, client: &vnc::VncClient) {
        let Some(pending) = self.pending_photo.as_mut() else {
            return;
        };
        let Some(token) = pending.token.clone() else {
            return;
        };
        if pending.id != 0 || Instant::now() < pending.poll_due {
            return;
        }
        let id = self.next_management_id;
        self.next_management_id = self.next_management_id.wrapping_add(1);
        pending.id = id;
        if let Err(error) = client
            .input(X11Event::TightFile(TightFileCommand::Manage {
                op: 7,
                id,
                path: token,
                destination: None,
            }))
            .await
        {
            self.photo_fail(error.to_string());
        }
    }

    fn photo_result(&mut self, op: u8, id: u32, status: u8, message: String) {
        if op == 8 {
            if self.pending_cleanup_id.take() != Some(id) {
                return;
            }
            // Cleanup is best effort; the import/export result remains visible.
            return;
        }
        let Some(mut pending) = self.pending_photo.take() else {
            return;
        };
        let expected_op = if pending.token.is_some() {
            7
        } else {
            pending.op
        };
        if pending.id != id || expected_op != op {
            self.photo_fail("Unexpected Photos response");
            return;
        }
        if status == 1 {
            self.photo_fail(message);
            return;
        }
        if pending.token.is_none() {
            if status != 2 || message.len() != 36 {
                self.photo_fail("Invalid Photos job response");
                return;
            }
            pending.token = Some(message);
            pending.id = 0;
            pending.poll_due = Instant::now();
            self.pending_photo = Some(pending);
            return;
        }
        if status == 2 {
            pending.id = 0;
            pending.poll_due = Instant::now() + Duration::from_millis(200);
            self.pending_photo = Some(pending);
            return;
        }
        if status != 0 {
            self.photo_fail("Invalid Photos job status");
            return;
        }
        let Ok(json) = serde_json::from_str::<Value>(&message) else {
            self.photo_fail("Invalid Photos result");
            return;
        };
        match pending.action {
            PhotoAction::List { offset, album } => {
                let Some(rows) = json.get("entries").and_then(Value::as_array) else {
                    self.photo_fail("Invalid Photos list");
                    return;
                };
                let mut entries = Vec::with_capacity(rows.len());
                let albums = json
                    .get("albums")
                    .and_then(Value::as_array)
                    .map(|rows| {
                        rows.iter()
                            .filter_map(|row| {
                                Some(PhotoAlbum {
                                    id: row.get("id")?.as_str()?.to_owned(),
                                    name: row.get("name")?.as_str()?.to_owned(),
                                })
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                for row in rows {
                    let Some(id) = row.get("id").and_then(Value::as_str) else {
                        continue;
                    };
                    let Some(name) = row.get("name").and_then(Value::as_str) else {
                        continue;
                    };
                    let thumbnail = row
                        .get("thumbnail")
                        .and_then(Value::as_str)
                        .and_then(|data| {
                            base64::engine::general_purpose::STANDARD.decode(data).ok()
                        })
                        .unwrap_or_default();
                    entries.push(PhotoEntry {
                        id: id.to_owned(),
                        name: name.to_owned(),
                        created: row.get("date").and_then(Value::as_i64).unwrap_or_default(),
                        width: row
                            .get("width")
                            .and_then(Value::as_u64)
                            .unwrap_or_default()
                            .min(u32::MAX as u64) as u32,
                        height: row
                            .get("height")
                            .and_then(Value::as_u64)
                            .unwrap_or_default()
                            .min(u32::MAX as u64) as u32,
                        thumbnail,
                    });
                }
                let total = json
                    .get("total")
                    .and_then(Value::as_u64)
                    .unwrap_or_default() as usize;
                let next = match self.pending_photo_page.as_mut() {
                    Some(page) if page.album == album => {
                        page.append(offset, rows.len(), entries, albums, total)
                    }
                    _ => Err(()),
                };
                match next {
                    Ok(Some(next_offset)) => {
                        self.followup.push_back(FileCommand::PhotoList {
                            offset: next_offset,
                            album,
                        });
                        return;
                    }
                    Ok(None) => {}
                    Err(()) => {
                        self.photo_fail("Photos page changed while loading");
                        return;
                    }
                }
                let page = self.pending_photo_page.take().unwrap();
                file_transfer::update(&self.state, |state| {
                    state.photo_entries = page.entries;
                    state.photo_albums = page.albums;
                    state.photo_album = page.album;
                    state.photo_offset = page.offset;
                    state.photo_total = page.total;
                    state.photo_busy = false;
                    if state.photo_status.as_deref() == Some("Loading Photos…") {
                        state.photo_status = None;
                    }
                    state.photo_revision = state.photo_revision.wrapping_add(1);
                });
            }
            PhotoAction::Import { restore_path } => {
                let Some(asset_id) = json.get("assetId").and_then(Value::as_str) else {
                    self.photo_fail("Photos did not return an asset ID");
                    return;
                };
                self.pending_photo_upload = None;
                file_transfer::update(&self.state, |state| {
                    // Keep Photos busy until the follow-up list finishes, so a
                    // queued upload cannot start while the refresh is in flight.
                    state.photo_busy = true;
                    state.photo_status = Some(format!("Imported into Photos · {asset_id}"));
                    state.photo_revision = state.photo_revision.wrapping_add(1);
                });
                if let Some(path) = restore_path {
                    self.followup.push_back(FileCommand::List(path));
                }
                self.followup.push_back(FileCommand::PhotoList {
                    offset: 0,
                    album: None,
                });
            }
            PhotoAction::Export { destination } => {
                let Some(remote) = json.get("path").and_then(Value::as_str) else {
                    self.photo_fail("Photos did not return an export path");
                    return;
                };
                let Some(size) = json.get("size").and_then(Value::as_u64) else {
                    self.photo_fail("Photos did not return an export size");
                    return;
                };
                if !valid_remote_path(remote) || size == 0 || size > u32::MAX as u64 {
                    self.photo_fail("Invalid exported photo");
                    return;
                }
                self.pending_export_cleanup = Some(remote.to_owned());
                file_transfer::update(&self.state, |state| {
                    state.photo_status = Some("Downloading original photo…".into());
                });
                self.followup.push_back(FileCommand::Download {
                    remote: remote.to_owned(),
                    destination,
                    size,
                });
            }
            PhotoAction::Delete { album, count } => {
                let deleted_count = json.get("deleted").and_then(|value| {
                    value
                        .as_array()
                        .map(Vec::len)
                        .or_else(|| value.as_str().map(|_| 1))
                });
                if deleted_count != Some(count) {
                    self.photo_fail("Photos did not confirm deletion");
                    return;
                }
                file_transfer::update(&self.state, |state| {
                    state.photo_busy = false;
                    state.photo_status = Some(format!(
                        "Deleted {count} photo{}",
                        if count == 1 { "" } else { "s" }
                    ));
                    state.photo_revision = state.photo_revision.wrapping_add(1);
                });
                self.followup
                    .push_back(FileCommand::PhotoList { offset: 0, album });
            }
        }
    }

    async fn command(&mut self, client: &vnc::VncClient, command: FileCommand) {
        let caps = self.state.lock().unwrap().caps.unwrap_or_default();
        match command {
            FileCommand::List(path) => {
                if !caps.list || !valid_remote_path(&path) {
                    self.fail("Remote file browsing is unavailable");
                    return;
                }
                file_transfer::update(&self.state, |state| {
                    state.remote_path = path.clone();
                    state.listed_path = None;
                    state.listing = true;
                    state.error = None;
                    state.entries.clear();
                });
                self.pending_lists.push_back(path.clone());
                if let Err(error) = client
                    .input(X11Event::TightFile(TightFileCommand::List(path)))
                    .await
                {
                    self.pending_lists.pop_back();
                    self.fail(error.to_string());
                }
            }
            FileCommand::Download {
                remote,
                destination,
                size,
            } => {
                if !caps.download || !valid_remote_path(&remote) {
                    self.fail_download(&remote, "Remote download is unavailable");
                    return;
                }
                if self.state.lock().unwrap().download_blocked {
                    self.fail_download(
                        &remote,
                        "Reconnect before another download after an interrupted transfer",
                    );
                    return;
                }
                if self.active.is_some()
                    || self.pending_upload.is_some()
                    || self.pending_checksum.is_some()
                    || self
                        .state
                        .lock()
                        .unwrap()
                        .transfers
                        .iter()
                        .any(|record| matches!(record.status, TransferStatus::Sent))
                {
                    self.fail_download(
                        &remote,
                        "Wait for the current transfer verification to finish",
                    );
                    return;
                }
                match OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&destination)
                {
                    Ok(file) => {
                        let index = self.record(
                            destination
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned(),
                            TransferDirection::Download,
                            Some(size),
                        );
                        self.active = Some(ActiveFile::Download {
                            file,
                            remote: remote.clone(),
                            destination,
                            index,
                            hasher: Hasher::new(MessageDigest::sha256())
                                .expect("SHA-256 is available"),
                        });
                        if let Err(error) = client
                            .input(X11Event::TightFile(TightFileCommand::Download(
                                remote.clone(),
                            )))
                            .await
                        {
                            self.stop(TransferStatus::Failed(error.to_string()));
                            self.fail_download(&remote, error.to_string());
                        }
                    }
                    Err(error) => {
                        self.fail_download(&remote, format!("Cannot create download file: {error}"))
                    }
                }
            }
            FileCommand::Upload { source, remote } => {
                if !self.allow_upload {
                    self.fail("Uploading is disabled in view-only mode");
                    return;
                }
                if !caps.upload || !valid_remote_path(&remote) {
                    self.fail("Remote upload is unavailable");
                    return;
                }
                if self.active.is_some()
                    || self.pending_upload.is_some()
                    || self.pending_checksum.is_some()
                    || self
                        .state
                        .lock()
                        .unwrap()
                        .transfers
                        .iter()
                        .any(|record| matches!(record.status, TransferStatus::Sent))
                {
                    self.fail("Wait for the current transfer verification to finish");
                    return;
                }
                let (parent, name) = remote.rsplit_once('/').unwrap_or(("", ""));
                let parent = if parent.is_empty() { "/" } else { parent };
                let upload_error = {
                    let listed = self.state.lock().unwrap();
                    if listed.listed_path.as_deref() != Some(parent) {
                        Some("Wait for the remote folder to load before uploading")
                    } else if listed.entries.iter().any(|entry| entry.name == name) {
                        Some("A remote file with this name already exists")
                    } else {
                        None
                    }
                };
                if let Some(error) = upload_error {
                    self.fail(error);
                    return;
                }
                match File::open(&source).and_then(|file| Ok((file.metadata()?, file))) {
                    Ok((metadata, file)) if metadata.is_file() => {
                        let modified = metadata
                            .modified()
                            .ok()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map_or(0, |d| d.as_secs().min(u32::MAX as u64) as u32);
                        let index = self.record(
                            source
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned(),
                            TransferDirection::Upload,
                            Some(metadata.len()),
                        );
                        self.active = Some(ActiveFile::Upload {
                            file,
                            remote: remote.clone(),
                            modified,
                            index,
                            hasher: Hasher::new(MessageDigest::sha256())
                                .expect("SHA-256 is available"),
                        });
                        if let Err(error) = client
                            .input(X11Event::TightFile(TightFileCommand::Upload(remote)))
                            .await
                        {
                            self.stop(TransferStatus::Failed(error.to_string()));
                        }
                    }
                    Ok(_) => self.fail("Select a regular file to upload"),
                    Err(error) => self.fail(format!("Cannot read upload file: {error}")),
                }
            }
            FileCommand::Cancel => {
                let was_download = matches!(self.active, Some(ActiveFile::Download { .. }));
                if self.active.is_some() {
                    let command = if was_download {
                        TightFileCommand::CancelDownload
                    } else {
                        TightFileCommand::FailUpload
                    };
                    let _ = client.input(X11Event::TightFile(command)).await;
                    self.stop(TransferStatus::Cancelled);
                    if was_download {
                        self.block_download(
                            "Download cancelled; reconnect before downloading again",
                        );
                    }
                }
            }
            FileCommand::Delete(path) => self.manage(client, 1, path, None).await,
            FileCommand::CreateFolder(path) => self.manage(client, 2, path, None).await,
            FileCommand::Rename { from, to } => self.manage(client, 3, from, Some(to)).await,
            FileCommand::Replace { from, to } => self.manage(client, 10, from, Some(to)).await,
            FileCommand::Checksum {
                remote,
                index,
                expected,
                destination,
            } => {
                if !self.state.lock().unwrap().can_checksum || !valid_remote_path(&remote) {
                    return;
                }
                let id = self.next_management_id;
                self.next_management_id = self.next_management_id.wrapping_add(1);
                self.pending_checksum = Some(PendingChecksum {
                    id,
                    index,
                    expected,
                    destination,
                });
                if let Err(error) = client
                    .input(X11Event::TightFile(TightFileCommand::Manage {
                        op: 11,
                        id,
                        path: remote,
                        destination: None,
                    }))
                    .await
                {
                    if let Some(pending) = self.pending_checksum.take()
                        && let Some(destination) = pending.destination
                    {
                        let _ = std::fs::remove_file(destination);
                    }
                    file_transfer::update(&self.state, |state| {
                        if let Some(record) = state.transfers.get_mut(index) {
                            record.status = TransferStatus::Failed(error.to_string());
                        }
                    });
                }
            }
            FileCommand::PhotoList { offset, album } => {
                if self.pending_photo_upload.is_none() {
                    if self.pending_photo.is_some() {
                        file_transfer::update(&self.state, |state| {
                            state.photo_error =
                                Some("Wait for the current photo operation to finish".into());
                        });
                        return;
                    }
                    if let Some(page) = &self.pending_photo_page {
                        if page.next_offset != offset || page.album != album {
                            file_transfer::update(&self.state, |state| {
                                state.photo_error =
                                    Some("Wait for the current Photos page to finish".into());
                            });
                            return;
                        }
                    } else {
                        self.pending_photo_page =
                            Some(PendingPhotoPage::new(offset, album.clone()));
                    }
                    let request = if let Some(id) = &album {
                        serde_json::json!({"offset": offset, "album": id}).to_string()
                    } else {
                        offset.to_string()
                    };
                    self.start_photo(
                        client,
                        5,
                        request,
                        None,
                        PhotoAction::List { offset, album },
                    )
                    .await;
                }
            }
            FileCommand::PhotoImport {
                remote,
                expected_sha256,
            } => {
                if !valid_remote_path(&remote) {
                    self.photo_fail("Invalid remote image path");
                    return;
                }
                let restore_path = self
                    .pending_photo_upload
                    .as_ref()
                    .filter(|upload| upload.remote == remote)
                    .map(|upload| upload.restore_path.clone());
                self.start_photo(
                    client,
                    4,
                    remote,
                    expected_sha256,
                    PhotoAction::Import { restore_path },
                )
                .await;
            }
            FileCommand::PhotoExport {
                asset_id,
                destination,
            } => {
                self.start_photo(
                    client,
                    6,
                    asset_id,
                    None,
                    PhotoAction::Export { destination },
                )
                .await;
            }
            FileCommand::PhotoDelete(ids) => {
                let (can_delete, can_batch, album) = {
                    let state = self.state.lock().unwrap();
                    (
                        state.can_photo_delete,
                        state.can_photo_batch_delete,
                        state.photo_album.clone(),
                    )
                };
                if !can_delete || (ids.len() > 1 && !can_batch) {
                    self.photo_fail("Deleting photos is unavailable on this TrollVNC connection");
                    return;
                }
                if ids.is_empty() || ids.len() > 50 || ids.iter().any(|id| id.is_empty()) {
                    self.photo_fail("Select between 1 and 50 photos");
                    return;
                }
                let count = ids.len();
                let request = if count == 1 {
                    ids[0].clone()
                } else {
                    serde_json::json!({"ids": ids}).to_string()
                };
                self.start_photo(
                    client,
                    9,
                    request,
                    None,
                    PhotoAction::Delete { album, count },
                )
                .await;
            }
            FileCommand::UploadToPhotos(source) => {
                if !self.allow_upload || !self.state.lock().unwrap().can_photos {
                    self.photo_fail("Uploading to Photos is unavailable");
                    return;
                }
                if self.pending_photo_upload.is_some()
                    || self.pending_photo.is_some()
                    || self.active.is_some()
                    || self.pending_upload.is_some()
                    || self.pending_checksum.is_some()
                    || self
                        .state
                        .lock()
                        .unwrap()
                        .transfers
                        .iter()
                        .any(|record| matches!(record.status, TransferStatus::Sent))
                {
                    self.photo_fail("Wait for the current transfer to finish");
                    return;
                }
                let extension = source
                    .extension()
                    .and_then(|value| value.to_str())
                    .map(str::to_ascii_lowercase)
                    .unwrap_or_default();
                if !["png", "jpg", "jpeg", "heic", "heif"].contains(&extension.as_str()) {
                    self.photo_fail("Choose a PNG, JPEG, HEIC, or HEIF image");
                    return;
                }
                let Ok(mut file) = File::open(&source) else {
                    self.photo_fail("Cannot open the selected image");
                    return;
                };
                let Ok(metadata) = file.metadata() else {
                    self.photo_fail("Cannot read image metadata");
                    return;
                };
                if !metadata.is_file() || metadata.len() == 0 || metadata.len() > 200 * 1024 * 1024
                {
                    self.photo_fail("Image must be a regular file of at most 200 MB");
                    return;
                }
                let Ok(mut hasher) = Hasher::new(MessageDigest::sha256()) else {
                    self.photo_fail("Cannot start image checksum");
                    return;
                };
                let mut buffer = [0u8; 65536];
                loop {
                    match file.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(count) => {
                            if hasher.update(&buffer[..count]).is_err() {
                                self.photo_fail("Cannot hash image");
                                return;
                            }
                        }
                        Err(error) => {
                            self.photo_fail(format!("Cannot read image: {error}"));
                            return;
                        }
                    }
                }
                let Ok(digest) = hasher.finish() else {
                    self.photo_fail("Cannot finish image checksum");
                    return;
                };
                let Some(original_name) = source.file_name().and_then(|name| name.to_str()) else {
                    self.photo_fail("Image name is not valid UTF-8");
                    return;
                };
                if original_name.len() > 180 {
                    self.photo_fail("Image name is too long");
                    return;
                }
                let hash = digest
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                let remote = format!(
                    "/Media/DCIM/.MISC/Incoming/rv-upload-{}--{}",
                    Uuid::new_v4(),
                    original_name
                );
                let restore_path = self.state.lock().unwrap().remote_path.clone();
                self.pending_photo_upload = Some(PendingPhotoUpload {
                    remote,
                    expected_sha256: hash,
                    restore_path,
                });
                file_transfer::update(&self.state, |state| {
                    state.photo_busy = true;
                    state.photo_error = None;
                    state.photo_status = Some("Uploading to Photos…".into());
                });
                self.followup
                    .push_back(FileCommand::List("/Media/DCIM/.MISC/Incoming".into()));
                // The source is retained until the staging listing arrives.
                self.photo_upload_source = Some(source);
            }
            FileCommand::PhotoCleanupExport(remote) => {
                let id = self.next_management_id;
                self.next_management_id = self.next_management_id.wrapping_add(1);
                self.pending_cleanup_id = Some(id);
                if let Err(error) = client
                    .input(X11Event::TightFile(TightFileCommand::Manage {
                        op: 8,
                        id,
                        path: remote,
                        destination: None,
                    }))
                    .await
                {
                    self.photo_fail(format!("Cannot clean exported photo: {error}"));
                }
            }
        }
    }

    async fn manage(
        &mut self,
        client: &vnc::VncClient,
        op: u8,
        path: String,
        destination: Option<String>,
    ) {
        let allowed = {
            let state = self.state.lock().unwrap();
            match op {
                1 => state.can_delete,
                2 => state.can_mkdir,
                3 => state.can_rename,
                10 => state.can_replace,
                _ => false,
            }
        };
        if !self.allow_upload
            || !allowed
            || !valid_remote_path(&path)
            || path == "/"
            || path.len() > 4096
        {
            self.fail("Remote file management is unavailable");
            return;
        }
        if self.active.is_some()
            || self.pending_management.is_some()
            || self.pending_upload.is_some()
            || self.pending_checksum.is_some()
            || self
                .state
                .lock()
                .unwrap()
                .transfers
                .iter()
                .any(|record| matches!(record.status, TransferStatus::Sent))
        {
            self.fail("Wait for the current file operation to finish");
            return;
        }
        let (parent, name) = path.rsplit_once('/').unwrap_or(("", ""));
        let parent = if parent.is_empty() { "/" } else { parent };
        if !valid_remote_name(name) {
            self.fail("Invalid remote file name");
            return;
        }
        let dest_name = if let Some(destination) = &destination {
            let (dest_parent, dest_name) = destination.rsplit_once('/').unwrap_or(("", ""));
            let dest_parent = if dest_parent.is_empty() {
                "/"
            } else {
                dest_parent
            };
            if !valid_remote_path(destination)
                || destination.len() > 4096
                || !valid_remote_name(dest_name)
                || dest_parent != parent
                || dest_name == name
            {
                self.fail("Enter a different name in the same remote folder");
                return;
            }
            Some(dest_name)
        } else {
            None
        };
        {
            let state = self.state.lock().unwrap();
            if state.listed_path.as_deref() != Some(parent) {
                drop(state);
                self.fail("Load the remote folder before changing it");
                return;
            }
            if (op == 1 || op == 3 || op == 10)
                && !state.entries.iter().any(|entry| entry.name == name)
                || op == 2 && state.entries.iter().any(|entry| entry.name == name)
                || op == 3
                    && dest_name.is_some_and(|dest_name| {
                        state.entries.iter().any(|entry| entry.name == dest_name)
                    })
                || op == 10
                    && !dest_name.is_some_and(|dest_name| {
                        state
                            .entries
                            .iter()
                            .any(|entry| entry.name == dest_name && !entry.is_dir)
                    })
            {
                drop(state);
                self.fail("The remote folder contents changed; refresh and retry");
                return;
            }
        }
        let id = self.next_management_id;
        self.next_management_id = self.next_management_id.wrapping_add(1);
        self.pending_management = Some((id, op, parent.to_owned()));
        file_transfer::update(&self.state, |state| {
            state.management_busy = true;
            state.error = None;
        });
        if let Err(error) = client
            .input(X11Event::TightFile(TightFileCommand::Manage {
                op,
                id,
                path,
                destination,
            }))
            .await
        {
            self.pending_management = None;
            file_transfer::update(&self.state, |state| state.management_busy = false);
            self.fail(error.to_string());
        }
    }

    fn record(&mut self, name: String, direction: TransferDirection, total: Option<u64>) -> usize {
        let mut index = 0;
        file_transfer::update(&self.state, |state| {
            state.error = None;
            state.transfers.push(TransferRecord {
                name,
                direction,
                bytes: 0,
                total,
                status: TransferStatus::Running,
            });
            index = state.transfers.len() - 1;
        });
        index
    }

    fn stop(&mut self, status: TransferStatus) {
        if let Some(active) = self.active.take() {
            let photo_upload_failed = matches!(
                &status,
                TransferStatus::Failed(_) | TransferStatus::Cancelled
            ) && matches!(&active, ActiveFile::Upload { remote, .. }
                    if self.pending_photo_upload.as_ref().is_some_and(|upload| upload.remote == *remote));
            let index = match &active {
                ActiveFile::Upload { index, .. } | ActiveFile::Download { index, .. } => *index,
            };
            if let ActiveFile::Download {
                file, destination, ..
            } = active
            {
                drop(file);
                if !matches!(status, TransferStatus::Complete | TransferStatus::Sent) {
                    let _ = std::fs::remove_file(destination);
                }
            }
            file_transfer::update(&self.state, |state| {
                if let Some(record) = state.transfers.get_mut(index) {
                    record.status = status;
                }
            });
            if photo_upload_failed {
                self.photo_fail("Photo upload failed");
            }
        }
    }

    fn block_download(&mut self, message: impl Into<String>) {
        let message = message.into();
        file_transfer::update(&self.state, |state| {
            state.download_blocked = true;
            state.error = Some(message);
        });
    }

    async fn advance_upload(&mut self, client: &vnc::VncClient) {
        let Some(ActiveFile::Upload {
            file,
            remote,
            modified,
            index,
            hasher,
        }) = self.active.as_mut()
        else {
            return;
        };
        let mut bytes = vec![0u8; 8192];
        match file.read(&mut bytes) {
            Ok(0) => {
                let modified = *modified;
                let remote = remote.clone();
                let index = *index;
                let expected_sha256 = match hasher.finish() {
                    Ok(digest) => digest
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<String>(),
                    Err(error) => {
                        self.stop(TransferStatus::Failed(error.to_string()));
                        return;
                    }
                };
                let incomplete = {
                    let state = self.state.lock().unwrap();
                    let record = &state.transfers[index];
                    record.total.is_some_and(|total| record.bytes != total)
                };
                if incomplete {
                    let _ = client
                        .input(X11Event::TightFile(TightFileCommand::FailUpload))
                        .await;
                    self.stop(TransferStatus::Failed(
                        "Upload source changed before transfer completed".into(),
                    ));
                    return;
                }
                if let Err(error) = client
                    .input(X11Event::TightFile(TightFileCommand::UploadEnd(modified)))
                    .await
                {
                    self.stop(TransferStatus::Failed(error.to_string()));
                } else {
                    self.stop(TransferStatus::Sent);
                    let (parent, name) = remote.rsplit_once('/').unwrap_or(("", ""));
                    let path = if parent.is_empty() { "/" } else { parent }.to_owned();
                    let size = self.state.lock().unwrap().transfers[index].bytes;
                    self.pending_upload = Some(PendingUploadVerification {
                        path: path.clone(),
                        name: name.to_owned(),
                        size,
                        index,
                        expected_sha256,
                    });
                    if self.state.lock().unwrap().remote_path == path {
                        self.command(client, FileCommand::List(path)).await;
                    }
                }
            }
            Ok(n) => {
                let exceeds_source = {
                    let state = self.state.lock().unwrap();
                    let record = &state.transfers[*index];
                    record
                        .total
                        .is_some_and(|total| record.bytes.saturating_add(n as u64) > total)
                };
                if exceeds_source {
                    let _ = client
                        .input(X11Event::TightFile(TightFileCommand::FailUpload))
                        .await;
                    self.stop(TransferStatus::Failed(
                        "Upload source changed before transfer completed".into(),
                    ));
                    return;
                }
                bytes.truncate(n);
                if let Err(error) = hasher.update(&bytes) {
                    self.stop(TransferStatus::Failed(error.to_string()));
                    return;
                }
                let index = *index;
                if let Err(error) = client
                    .input(X11Event::TightFile(TightFileCommand::UploadData(bytes)))
                    .await
                {
                    self.stop(TransferStatus::Failed(error.to_string()));
                } else {
                    file_transfer::update(&self.state, |state| {
                        state.transfers[index].bytes += n as u64;
                    });
                }
            }
            Err(error) => {
                let _ = client
                    .input(X11Event::TightFile(TightFileCommand::FailUpload))
                    .await;
                self.stop(TransferStatus::Failed(error.to_string()));
            }
        }
    }

    fn event(&mut self, event: TightFileEvent) {
        match event {
            TightFileEvent::Capabilities(caps) => {
                file_transfer::update(&self.state, |state| state.caps = Some(caps))
            }
            TightFileEvent::ManagementAvailable {
                delete,
                mkdir,
                rename,
                photos,
                photo_delete,
                photo_batch_delete,
                replace,
                checksum,
            } => {
                file_transfer::update(&self.state, |state| {
                    state.can_delete = delete;
                    state.can_mkdir = mkdir;
                    state.can_rename = rename;
                    state.can_photos = photos;
                    state.can_photo_delete = photo_delete;
                    state.can_photo_batch_delete = photo_batch_delete;
                    state.can_replace = replace;
                    state.can_checksum = checksum;
                });
            }
            TightFileEvent::ManagementResult {
                op,
                id,
                status,
                message,
            } => {
                if op == 11 {
                    if self
                        .pending_checksum
                        .as_ref()
                        .is_some_and(|pending| pending.id == id)
                        && let Some(pending) = self.pending_checksum.take()
                    {
                        let verified = status == 0 && message == pending.expected;
                        let is_upload = pending.destination.is_none();
                        let cleanup_error = if !verified {
                            pending.destination.as_ref().and_then(|destination| {
                                std::fs::remove_file(destination)
                                    .err()
                                    .filter(|error| error.kind() != std::io::ErrorKind::NotFound)
                                    .map(|error| {
                                        format!("; could not remove unverified download: {error}")
                                    })
                            })
                        } else {
                            None
                        };
                        file_transfer::update(&self.state, |state| {
                            if let Some(record) = state.transfers.get_mut(pending.index) {
                                record.status = if verified {
                                    TransferStatus::Verified
                                } else {
                                    TransferStatus::Failed(format!(
                                        "{}{}",
                                        if status == 0 {
                                            "Remote SHA-256 differs from local file".into()
                                        } else {
                                            message.clone()
                                        },
                                        cleanup_error.unwrap_or_default()
                                    ))
                                };
                            }
                        });
                        if verified
                            && is_upload
                            && let Some(upload) = &self.pending_photo_upload
                        {
                            self.followup.push_back(FileCommand::PhotoImport {
                                remote: upload.remote.clone(),
                                expected_sha256: Some(upload.expected_sha256.clone()),
                            });
                        } else if !verified && is_upload && self.pending_photo_upload.is_some() {
                            self.photo_fail("Photo upload checksum did not match");
                        }
                    }
                    return;
                }
                if (4..=9).contains(&op) {
                    self.photo_result(op, id, status, message);
                    return;
                }
                let Some((pending_id, pending_op, parent)) = self.pending_management.take() else {
                    return;
                };
                if pending_id != id || pending_op != op {
                    file_transfer::update(&self.state, |state| state.management_busy = false);
                    self.fail("Unexpected file management response");
                    return;
                }
                file_transfer::update(&self.state, |state| {
                    state.management_busy = false;
                    state.error = (status != 0).then_some(message);
                    if state.error.is_none() && state.remote_path == parent {
                        state.listed_path = None;
                        state.management_revision = state.management_revision.wrapping_add(1);
                    }
                });
            }
            TightFileEvent::List {
                failed,
                mut entries,
            } => {
                let Some(path) = self.pending_lists.pop_front() else {
                    return;
                };
                entries.retain(|entry| valid_remote_name(&entry.name));
                entries.sort_by(|a, b| {
                    b.is_dir
                        .cmp(&a.is_dir)
                        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                });
                let verified_photo_upload = !failed
                    && self.pending_photo_upload.as_ref().is_some_and(|upload| {
                        let (parent, name) = upload.remote.rsplit_once('/').unwrap_or(("", ""));
                        parent == path
                            && entries
                                .iter()
                                .any(|entry| entry.name == name && !entry.is_dir)
                    })
                    && self.pending_upload.as_ref().is_some_and(|pending| {
                        pending.path == path
                            && self
                                .pending_photo_upload
                                .as_ref()
                                .is_some_and(|upload| upload.remote.ends_with(&pending.name))
                    });
                if !failed
                    && self.pending_upload.as_ref().is_some_and(|pending| {
                        pending.path == path
                            && entries.iter().any(|entry| {
                                entry.name == pending.name
                                    && !entry.is_dir
                                    && entry.size as u64 == pending.size
                            })
                    })
                    && let Some(pending) = self.pending_upload.take()
                {
                    if self.state.lock().unwrap().can_checksum {
                        self.followup.push_back(FileCommand::Checksum {
                            remote: if path == "/" {
                                format!("/{}", pending.name)
                            } else {
                                format!("{}/{}", path, pending.name)
                            },
                            index: pending.index,
                            expected: pending.expected_sha256,
                            destination: None,
                        });
                    } else {
                        file_transfer::update(&self.state, |state| {
                            if let Some(record) = state.transfers.get_mut(pending.index)
                                && matches!(record.status, TransferStatus::Sent)
                            {
                                record.status = TransferStatus::Complete;
                            }
                        });
                    }
                    if !self.state.lock().unwrap().can_checksum
                        && verified_photo_upload
                        && let Some(upload) = &self.pending_photo_upload
                    {
                        self.followup.push_back(FileCommand::PhotoImport {
                            remote: upload.remote.clone(),
                            expected_sha256: Some(upload.expected_sha256.clone()),
                        });
                    }
                } else if self
                    .pending_upload
                    .as_ref()
                    .is_some_and(|pending| pending.path == path)
                    && let Some(pending) = self.pending_upload.take()
                {
                    file_transfer::update(&self.state, |state| {
                        if let Some(record) = state.transfers.get_mut(pending.index) {
                            record.status = TransferStatus::Failed(if failed {
                                "Could not list the uploaded file for verification".into()
                            } else {
                                "Uploaded file size does not match the source".into()
                            });
                        }
                    });
                    if self.pending_photo_upload.is_some() {
                        self.photo_fail("Photo upload could not be verified");
                    }
                }
                if path == "/Media/DCIM/.MISC/Incoming"
                    && let Some(source) = self.photo_upload_source.take()
                {
                    if failed {
                        self.photo_fail("Cannot open Photos staging folder");
                    } else if let Some(upload) = &self.pending_photo_upload {
                        let remote_name = upload.remote.rsplit('/').next().unwrap_or("");
                        if entries.iter().any(|entry| entry.name == remote_name) {
                            self.photo_fail("Photos staging name already exists");
                        } else {
                            self.followup.push_back(FileCommand::Upload {
                                source,
                                remote: upload.remote.clone(),
                            });
                        }
                    }
                }
                if path != self.state.lock().unwrap().remote_path {
                    return;
                }
                file_transfer::update(&self.state, |state| {
                    state.listing = false;
                    state.entries = entries;
                    state.listed_path = (!failed).then_some(path);
                    state.error = failed.then(|| "Cannot list this remote folder".into());
                });
            }
            TightFileEvent::DownloadData(data) => {
                if let Some(ActiveFile::Download {
                    file,
                    index,
                    hasher,
                    ..
                }) = self.active.as_mut()
                {
                    let oversized = {
                        let state = self.state.lock().unwrap();
                        let record = &state.transfers[*index];
                        record.total.is_some_and(|total| {
                            record.bytes.saturating_add(data.len() as u64) > total
                        })
                    };
                    if oversized {
                        self.stop(TransferStatus::Failed(
                            "Download exceeded the remote file size".into(),
                        ));
                        self.block_download("Download exceeded its advertised size; reconnect before downloading again");
                        return;
                    }
                    match file.write_all(&data) {
                        Ok(()) => {
                            if let Err(error) = hasher.update(&data) {
                                self.stop(TransferStatus::Failed(error.to_string()));
                                self.block_download(
                                    "Download checksum failed; reconnect before downloading again",
                                );
                                return;
                            }
                            let index = *index;
                            file_transfer::update(&self.state, |state| {
                                state.transfers[index].bytes += data.len() as u64
                            });
                        }
                        Err(error) => {
                            self.stop(TransferStatus::Failed(error.to_string()));
                            self.block_download(
                                "Download write failed; reconnect before downloading again",
                            );
                        }
                    }
                }
            }
            TightFileEvent::DownloadEnd { .. } => {
                let short = if let Some(ActiveFile::Download { index, .. }) = &self.active {
                    let state = self.state.lock().unwrap();
                    let record = &state.transfers[*index];
                    record.total.is_some_and(|total| record.bytes != total)
                } else {
                    false
                };
                if short {
                    self.stop(TransferStatus::Failed(
                        "Download size did not match the remote file".into(),
                    ));
                } else {
                    let checksum = if self.pending_export_cleanup.is_none()
                        && self.state.lock().unwrap().can_checksum
                    {
                        if let Some(ActiveFile::Download {
                            file,
                            remote,
                            destination,
                            index,
                            hasher,
                        }) = self.active.as_mut()
                        {
                            match file
                                .sync_all()
                                .and_then(|_| hasher.finish().map_err(std::io::Error::other))
                            {
                                Ok(digest) => Some((
                                    remote.clone(),
                                    destination.clone(),
                                    *index,
                                    digest
                                        .iter()
                                        .map(|byte| format!("{byte:02x}"))
                                        .collect::<String>(),
                                )),
                                Err(error) => {
                                    self.stop(TransferStatus::Failed(format!(
                                        "Cannot finish download: {error}"
                                    )));
                                    return;
                                }
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    if let Some((remote, destination, index, expected)) = checksum {
                        self.stop(TransferStatus::Sent);
                        self.followup.push_back(FileCommand::Checksum {
                            remote,
                            index,
                            expected,
                            destination: Some(destination),
                        });
                    } else {
                        self.stop(TransferStatus::Complete);
                    }
                }
                if let Some(remote) = self.pending_export_cleanup.take() {
                    self.followup
                        .push_back(FileCommand::PhotoCleanupExport(remote));
                    if short {
                        self.photo_fail("Original photo size did not match");
                    } else {
                        file_transfer::update(&self.state, |state| {
                            state.photo_busy = false;
                            state.photo_status = Some("Original photo downloaded".into());
                            state.photo_revision = state.photo_revision.wrapping_add(1);
                        });
                    }
                }
            }
            TightFileEvent::DownloadFailed(reason) => {
                if matches!(self.active, Some(ActiveFile::Download { .. })) {
                    self.stop(TransferStatus::Failed(reason.clone()));
                }
                self.fail(reason);
                if let Some(remote) = self.pending_export_cleanup.take() {
                    self.followup
                        .push_back(FileCommand::PhotoCleanupExport(remote));
                    self.photo_fail("Original photo download failed");
                }
            }
            TightFileEvent::UploadCancelled(reason) => {
                if matches!(self.active, Some(ActiveFile::Upload { .. })) {
                    self.stop(TransferStatus::Failed(reason.clone()));
                } else {
                    let pending = self.pending_upload.take();
                    file_transfer::update(&self.state, |state| {
                        let index = pending.map(|pending| pending.index).or_else(|| {
                            state.transfers.iter().rposition(|record| {
                                matches!(record.direction, TransferDirection::Upload)
                            })
                        });
                        let record = index.and_then(|index| state.transfers.get_mut(index));
                        if let Some(record) = record
                            && matches!(record.direction, TransferDirection::Upload)
                        {
                            record.status = TransferStatus::Failed(reason.clone());
                        }
                    });
                }
                self.fail(reason);
                if self.pending_photo_upload.is_some() {
                    self.photo_fail("Photo upload failed");
                }
            }
        }
    }
}

impl Drop for FileRuntime {
    fn drop(&mut self) {
        self.stop(TransferStatus::Failed("Connection closed".into()));
        if let Some(pending) = self.pending_checksum.take() {
            if let Some(destination) = pending.destination {
                let _ = std::fs::remove_file(destination);
            }
            file_transfer::update(&self.state, |state| {
                if let Some(record) = state.transfers.get_mut(pending.index) {
                    record.status =
                        TransferStatus::Failed("Connection closed during verification".into());
                }
            });
        }
        if let Some(pending) = self.pending_upload.take() {
            file_transfer::update(&self.state, |state| {
                if let Some(record) = state.transfers.get_mut(pending.index) {
                    record.status = TransferStatus::Failed(
                        "Connection closed before upload verification".into(),
                    );
                }
            });
        }
        file_transfer::update(&self.state, |state| {
            state.caps = None;
            state.can_delete = false;
            state.can_mkdir = false;
            state.can_rename = false;
            state.can_replace = false;
            state.can_checksum = false;
            state.can_photos = false;
            state.management_busy = false;
            state.listing = false;
            state.error = Some("Connection closed".into());
        });
    }
}

fn valid_remote_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', '\0'])
}

fn valid_remote_path(path: &str) -> bool {
    path.starts_with('/')
        && !path.contains('\0')
        && path
            .split('/')
            .filter(|part| !part.is_empty())
            .all(valid_remote_name)
}

#[cfg(test)]
mod transport_tests {
    use super::*;

    #[test]
    fn photo_import_stays_busy_until_refresh_finishes() {
        let state = Arc::new(Mutex::new(FileTransferSnapshot {
            can_photos: true,
            photo_busy: true,
            ..Default::default()
        }));
        let mut runtime = FileRuntime::new(state.clone(), true);
        runtime.pending_photo = Some(PendingPhoto {
            id: 7,
            op: 4,
            token: Some("import-token".into()),
            poll_due: Instant::now(),
            action: PhotoAction::Import { restore_path: None },
        });
        runtime.photo_result(7, 7, 0, r#"{"assetId":"new-photo"}"#.into());
        {
            let snapshot = state.lock().unwrap();
            assert!(snapshot.photo_busy);
            assert!(
                snapshot
                    .photo_status
                    .as_deref()
                    .unwrap()
                    .starts_with("Imported into Photos")
            );
        }
        assert!(matches!(
            runtime.followup.back(),
            Some(FileCommand::PhotoList { .. })
        ));

        runtime.pending_photo = Some(PendingPhoto {
            id: 8,
            op: 5,
            token: Some("list-token".into()),
            poll_due: Instant::now(),
            action: PhotoAction::List {
                offset: 0,
                album: None,
            },
        });
        runtime.pending_photo_page = Some(PendingPhotoPage::new(0, None));
        runtime.photo_result(7, 8, 0, r#"{"entries":[],"total":0}"#.into());
        assert!(!state.lock().unwrap().photo_busy);
    }

    #[test]
    fn photo_list_combines_five_transport_replies_into_fifty_item_page() {
        let state = Arc::new(Mutex::new(FileTransferSnapshot {
            can_photos: true,
            photo_busy: true,
            ..Default::default()
        }));
        let mut runtime = FileRuntime::new(state.clone(), true);
        runtime.pending_photo_page = Some(PendingPhotoPage::new(0, None));
        for (chunk, offset) in [0, 12, 24, 36, 48].into_iter().enumerate() {
            let id = chunk as u32 + 1;
            runtime.pending_photo = Some(PendingPhoto {
                id,
                op: 5,
                token: Some(format!("page-{chunk}")),
                poll_due: Instant::now(),
                action: PhotoAction::List {
                    offset,
                    album: None,
                },
            });
            let rows = (offset..(offset + 12).min(56))
                .map(|index| {
                    serde_json::json!({
                        "id": format!("photo-{index}"),
                        "name": format!("IMG_{index:04}.JPG"),
                        "thumbnail": ""
                    })
                })
                .collect::<Vec<_>>();
            runtime.photo_result(
                7,
                id,
                0,
                serde_json::json!({"entries": rows, "total": 56, "albums": []}).to_string(),
            );
            let snapshot = state.lock().unwrap();
            if chunk < 4 {
                assert!(snapshot.photo_busy);
                assert!(snapshot.photo_entries.is_empty());
            } else {
                assert!(!snapshot.photo_busy);
                assert_eq!(snapshot.photo_entries.len(), PHOTO_PAGE_SIZE);
                assert_eq!(snapshot.photo_entries[49].id, "photo-49");
                assert_eq!(snapshot.photo_total, 56);
            }
        }
        assert!(runtime.pending_photo_page.is_none());

        runtime.pending_photo_page = Some(PendingPhotoPage::new(50, None));
        runtime.pending_photo = Some(PendingPhoto {
            id: 6,
            op: 5,
            token: Some("second-page".into()),
            poll_due: Instant::now(),
            action: PhotoAction::List {
                offset: 50,
                album: None,
            },
        });
        let rows = (50..56)
            .map(|index| {
                serde_json::json!({
                    "id": format!("photo-{index}"),
                    "name": format!("IMG_{index:04}.JPG")
                })
            })
            .collect::<Vec<_>>();
        runtime.photo_result(
            7,
            6,
            0,
            serde_json::json!({"entries": rows, "total": 56}).to_string(),
        );
        let snapshot = state.lock().unwrap();
        assert_eq!(snapshot.photo_offset, 50);
        assert_eq!(snapshot.photo_entries.len(), 6);
        assert_eq!(snapshot.photo_entries[0].id, "photo-50");
    }

    #[test]
    fn oversized_download_removes_partial_file_and_quarantines_stream() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("download.bin");
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
            .unwrap();
        let state = Arc::new(Mutex::new(FileTransferSnapshot::default()));
        let mut runtime = FileRuntime::new(state.clone(), true);
        let index = runtime.record("download.bin".into(), TransferDirection::Download, Some(3));
        runtime.active = Some(ActiveFile::Download {
            file,
            remote: "/download.bin".into(),
            destination: destination.clone(),
            index,
            hasher: Hasher::new(MessageDigest::sha256()).unwrap(),
        });
        runtime.event(TightFileEvent::DownloadData(b"abcd".to_vec()));
        runtime.event(TightFileEvent::DownloadEnd { modified: 0 });
        let snapshot = state.lock().unwrap();
        assert!(snapshot.download_blocked);
        assert!(matches!(
            snapshot.transfers[index].status,
            TransferStatus::Failed(_)
        ));
        assert!(!destination.exists());
    }

    #[test]
    fn failed_download_checksum_removes_untrusted_file() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("download.bin");
        std::fs::write(&destination, b"bad data").unwrap();
        let state = Arc::new(Mutex::new(FileTransferSnapshot::default()));
        let mut runtime = FileRuntime::new(state.clone(), true);
        let index = runtime.record("download.bin".into(), TransferDirection::Download, Some(8));
        runtime.pending_checksum = Some(PendingChecksum {
            id: 7,
            index,
            expected: "expected".into(),
            destination: Some(destination.clone()),
        });
        runtime.event(TightFileEvent::ManagementResult {
            op: 11,
            id: 7,
            status: 0,
            message: "different".into(),
        });
        assert!(!destination.exists());
        assert!(matches!(
            state.lock().unwrap().transfers[index].status,
            TransferStatus::Failed(_)
        ));
    }

    #[test]
    fn let_server_choose_prefers_plain_but_accepts_vencrypt_only() {
        assert_eq!(
            choose_transport(EncryptionMode::LetServerChoose, &[1, 2, 19], false).unwrap(),
            Transport::Plain
        );
        assert_eq!(
            choose_transport(EncryptionMode::LetServerChoose, &[19], false).unwrap(),
            Transport::VeNCrypt
        );
        assert_eq!(
            choose_transport(EncryptionMode::LetServerChoose, &[16, 33], false).unwrap(),
            Transport::Tight
        );
    }

    #[test]
    fn prefer_on_and_always() {
        assert_eq!(
            choose_transport(EncryptionMode::PreferOn, &[2, 19], false).unwrap(),
            Transport::VeNCrypt
        );
        assert_eq!(
            choose_transport(EncryptionMode::PreferOn, &[2], false).unwrap(),
            Transport::Plain
        );
        assert!(choose_transport(EncryptionMode::Always, &[2], false).is_err());
    }

    #[test]
    fn off_never_encrypts() {
        assert_eq!(
            choose_transport(EncryptionMode::Off, &[2, 19], false).unwrap(),
            Transport::Plain
        );
        assert!(choose_transport(EncryptionMode::Off, &[19], false).is_err());
    }
    #[test]
    fn apple_security_offer_selects_ard_without_weakening_required_tls() {
        let types = [30, 33, 36, 35];
        for mode in [
            EncryptionMode::LetServerChoose,
            EncryptionMode::PreferOn,
            EncryptionMode::Off,
        ] {
            assert_eq!(
                choose_transport(mode, &types, true).unwrap(),
                Transport::Ard
            );
            assert_eq!(
                choose_transport(mode, &types, false).unwrap(),
                Transport::Ard
            );
        }
        assert!(choose_transport(EncryptionMode::Always, &types, true).is_err());
        assert_eq!(
            choose_transport(EncryptionMode::LetServerChoose, &[2, 30], true).unwrap(),
            Transport::Ard
        );
        assert_eq!(
            choose_transport(EncryptionMode::LetServerChoose, &[2, 30], false).unwrap(),
            Transport::Plain
        );
        assert_eq!(
            choose_transport(EncryptionMode::PreferOn, &[2, 19, 30], true).unwrap(),
            Transport::VeNCrypt
        );
        assert_eq!(
            choose_transport(EncryptionMode::LetServerChoose, &[2, 16, 30], true).unwrap(),
            Transport::Ard
        );
    }
}
