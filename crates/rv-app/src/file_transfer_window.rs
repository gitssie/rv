//! Compact two-pane browser for the TightVNC 1.x transfer extension.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, Local};
use directories::UserDirs;
use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    Disableable as _, Icon, IconName, Sizable, StyledExt, TitleBar,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{ContextMenuExt, DropdownMenu as _, PopupMenuItem},
    v_flex,
};
use rv_core::{ConnectionId, TransferFolders};
use rv_session::{
    FileCommand, FileTransferClient, FileTransferSnapshot, PHOTO_PAGE_SIZE, PhotoAlbum, PhotoEntry,
    TransferDirection, TransferStatus,
};
use smallvec::SmallVec;

use crate::actions::{FileTransferFullscreen, FileTransferSelectAll};
use crate::app::AddressBookApp;

const BG: u32 = 0x171c22;
const PANEL: u32 = 0x1e252e;
const ROW_ALT: u32 = 0x222b35;
const LINE: u32 = 0x35404b;
const TEXT: u32 = 0xe8eef4;
const MUTED: u32 = 0x9aa9b8;
const SELECTED: u32 = 0x245a9a;

fn is_image_name(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| {
            ["png", "jpg", "jpeg", "heic", "heif"].contains(&value.to_ascii_lowercase().as_str())
        })
}

#[derive(Clone)]
struct LocalEntry {
    path: PathBuf,
    name: String,
    is_dir: bool,
    size_bytes: Option<u64>,
    modified_time: Option<SystemTime>,
    size: String,
    modified: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FileSide {
    Local,
    Remote,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortColumn {
    Name,
    Size,
    Modified,
}

#[derive(Clone, Copy)]
struct FileSort {
    column: SortColumn,
    ascending: bool,
}

impl Default for FileSort {
    fn default() -> Self {
        Self {
            column: SortColumn::Name,
            ascending: true,
        }
    }
}

#[derive(Clone)]
struct FileDetails {
    side: FileSide,
    name: String,
    path: String,
    is_dir: bool,
    size: Option<u64>,
    modified: Option<SystemTime>,
}

#[derive(Clone)]
enum QueueAction {
    Upload {
        source: PathBuf,
        remote: String,
        replace: Option<String>,
    },
    Download {
        remote: String,
        destination: PathBuf,
        size: u64,
        replace: Option<PathBuf>,
    },
    PhotoExport {
        asset_id: String,
        destination: PathBuf,
        replace: Option<PathBuf>,
    },
    PhotoUpload(PathBuf),
    DeleteLocal(PathBuf),
    DeleteRemote(String),
}

#[derive(Clone)]
enum QueueState {
    Queued,
    Running,
    Complete,
    Skipped,
    Failed(String),
    Cancelled,
}

#[derive(Clone)]
struct QueueJob {
    name: String,
    action: QueueAction,
    state: QueueState,
    transfer_index: Option<usize>,
    transfer_start: usize,
    started_revision: u64,
    management_revision: u64,
    photo_revision: u64,
    replace_sent: bool,
    cleanup_enqueued: bool,
}

impl QueueJob {
    fn new(name: String, action: QueueAction) -> Self {
        Self {
            name,
            action,
            state: QueueState::Queued,
            transfer_index: None,
            transfer_start: 0,
            started_revision: 0,
            management_revision: 0,
            photo_revision: 0,
            replace_sent: false,
            cleanup_enqueued: false,
        }
    }
}

#[derive(Clone, Copy)]
enum CollisionPolicy {
    Skip,
    Rename,
    Replace,
}

#[derive(Clone)]
pub struct TransferMemory {
    pub folders: Rc<RefCell<TransferFolders>>,
    pub address_book: Option<WeakEntity<AddressBookApp>>,
    pub connection_id: Option<ConnectionId>,
}

fn default_local_dir() -> PathBuf {
    UserDirs::new()
        .map(|user| {
            user.download_dir()
                .unwrap_or_else(|| user.home_dir())
                .to_path_buf()
        })
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

pub fn open(
    host: String,
    client: FileTransferClient,
    allow_upload: bool,
    memory: TransferMemory,
    cx: &mut AsyncApp,
) -> Result<WindowHandle<gpui_component::Root>> {
    let mut options = TitleBar::window_options();
    options.window_bounds = Some(WindowBounds::Windowed(Bounds {
        origin: point(px(120.), px(100.)),
        size: size(px(1000.), px(620.)),
    }));
    options.window_min_size = Some(size(px(760.), px(440.)));
    options.titlebar = Some(TitlebarOptions {
        title: Some(format!("File transfer — {host}").into()),
        appears_transparent: true,
        traffic_light_position: Some(point(px(9.), px(9.))),
    });
    cx.open_window(options, move |window, cx| {
        let view =
            cx.new(|cx| FileTransferView::new(host, client, allow_upload, memory, window, cx));
        cx.new(|cx| gpui_component::Root::new(view, window, cx))
    })
}

pub struct FileTransferView {
    host: String,
    local_dir: PathBuf,
    local_path_input: Entity<InputState>,
    remote_path_input: Entity<InputState>,
    local_back: Vec<PathBuf>,
    remote_back: Vec<String>,
    local_entries: Vec<LocalEntry>,
    local_selected: Option<PathBuf>,
    local_selected_paths: Vec<PathBuf>,
    local_anchor: Option<usize>,
    local_sort: FileSort,
    local_loading: bool,
    local_error: Option<String>,
    transfers_open: bool,
    queue: Vec<QueueJob>,
    current_job: Option<usize>,
    pending_conflicts: Option<Vec<QueueJob>>,
    pending_batch_delete: Option<FileSide>,
    queue_idle_since: Option<Instant>,
    auto_collapse_armed: bool,
    client: Option<FileTransferClient>,
    remote: FileTransferSnapshot,
    preferred_remote: Option<String>,
    memory: TransferMemory,
    remote_selected: Option<String>,
    remote_selected_names: Vec<String>,
    remote_anchor: Option<usize>,
    remote_sort: FileSort,
    active_side: FileSide,
    photo_mode: bool,
    photo_selected: Option<String>,
    photo_selected_ids: Vec<String>,
    photo_anchor: Option<(usize, usize)>,
    photo_thumbs: HashMap<String, Arc<RenderImage>>,
    allow_upload: bool,
    details: Option<FileDetails>,
    pending_delete: Option<FileDetails>,
    pending_rename: Option<FileDetails>,
    new_folder_side: Option<FileSide>,
    new_folder_input: Entity<InputState>,
    status_error: Option<String>,
    fullscreen_toolbar_open: bool,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl FileTransferView {
    fn new(
        host: String,
        client: FileTransferClient,
        allow_upload: bool,
        memory: TransferMemory,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let local_dir = memory
            .folders
            .borrow()
            .local
            .clone()
            .filter(|path| path.is_dir())
            .unwrap_or_else(default_local_dir);
        let local_path_input = cx.new(|cx| {
            let mut input = InputState::new(window, cx);
            input.set_value(local_dir.display().to_string(), window, cx);
            input
        });
        let remote = client.snapshot();
        let photo_mode = memory.folders.borrow().photo_mode && remote.can_photos;
        let preferred_remote = memory.folders.borrow().remote.clone();
        let initial_remote = preferred_remote
            .clone()
            .unwrap_or_else(|| remote.remote_path.clone());
        let remote_path_input = cx.new(|cx| {
            let mut input = InputState::new(window, cx);
            input.set_value(initial_remote.clone(), window, cx);
            input
        });
        let new_folder_input = cx.new(|cx| InputState::new(window, cx));
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let subscriptions = vec![
            cx.subscribe_in(
                &local_path_input,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        this.go_local(window, cx);
                    }
                },
            ),
            cx.subscribe_in(
                &remote_path_input,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        this.go_remote(window, cx);
                    }
                },
            ),
            cx.subscribe_in(
                &new_folder_input,
                window,
                |this, _, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        if this.pending_rename.is_some() {
                            this.rename_item(cx);
                        } else {
                            this.create_folder(cx);
                        }
                    }
                },
            ),
        ];
        let mut view = Self {
            host,
            local_dir,
            local_path_input,
            remote_path_input,
            local_back: Vec::new(),
            remote_back: Vec::new(),
            local_entries: Vec::new(),
            local_selected: None,
            local_selected_paths: Vec::new(),
            local_anchor: None,
            local_sort: FileSort::default(),
            local_loading: false,
            local_error: None,
            transfers_open: true,
            queue: Vec::new(),
            current_job: None,
            pending_conflicts: None,
            pending_batch_delete: None,
            queue_idle_since: None,
            auto_collapse_armed: false,
            remote,
            preferred_remote,
            memory,
            client: Some(client),
            remote_selected: None,
            remote_selected_names: Vec::new(),
            remote_anchor: None,
            remote_sort: FileSort::default(),
            active_side: FileSide::Remote,
            photo_mode,
            photo_selected: None,
            photo_selected_ids: Vec::new(),
            photo_anchor: None,
            photo_thumbs: HashMap::new(),
            allow_upload,
            details: None,
            pending_delete: None,
            pending_rename: None,
            new_folder_side: None,
            new_folder_input,
            status_error: None,
            fullscreen_toolbar_open: false,
            focus,
            _subscriptions: subscriptions,
        };
        view.reload_local(cx);
        if view.remote.caps.is_some_and(|caps| caps.list) {
            view.request_remote_list(initial_remote);
        }
        if view.photo_mode {
            view.request_photo_list();
        }
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                if this.update(cx, |this, cx| this.poll_remote(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        view
    }

    fn poll_remote(&mut self, cx: &mut Context<Self>) {
        let Some(client) = &self.client else {
            return;
        };
        let Some(snapshot) = client.snapshot_if_changed(self.remote.revision) else {
            self.advance_queue(cx);
            return;
        };
        let first_caps = self.remote.caps.is_none() && snapshot.caps.is_some_and(|caps| caps.list);
        let download_finished = snapshot.transfers.iter().enumerate().any(|(index, new)| {
            matches!(new.direction, TransferDirection::Download)
                && matches!(
                    new.status,
                    TransferStatus::Complete | TransferStatus::Verified
                )
                && !self.remote.transfers.get(index).is_some_and(|old| {
                    matches!(old.direction, TransferDirection::Download)
                        && matches!(
                            old.status,
                            TransferStatus::Complete | TransferStatus::Verified
                        )
                        && old.name == new.name
                })
        });
        let management_finished = self.remote.management_revision != snapshot.management_revision
            && snapshot.error.is_none();
        let photos_changed = self.remote.photo_revision != snapshot.photo_revision;
        let thumbnails_changed = photos_changed
            && (snapshot.photo_entries.len() != self.remote.photo_entries.len()
                || snapshot
                    .photo_entries
                    .iter()
                    .zip(&self.remote.photo_entries)
                    .any(|(new, old)| new.id != old.id || new.thumbnail != old.thumbnail));
        let photo_page_changed = self.remote.photo_offset != snapshot.photo_offset
            || self.remote.photo_album != snapshot.photo_album;
        self.remote = snapshot;
        self.sync_photo_mode(cx);
        self.sort_remote_entries();
        if photos_changed {
            if photo_page_changed
                || self
                    .remote
                    .photo_status
                    .as_deref()
                    .is_some_and(|status| status.starts_with("Deleted "))
            {
                self.photo_selected = None;
                self.photo_selected_ids.clear();
                self.photo_anchor = None;
            }
            if thumbnails_changed {
                self.reconcile_photo_thumbs(cx);
            }
        }
        if self.remote.listed_path.as_deref() == Some(self.remote.remote_path.as_str())
            && self.remote_selected.as_ref().is_some_and(|selected| {
                !self
                    .remote
                    .entries
                    .iter()
                    .any(|entry| entry.name == *selected)
            })
        {
            self.remote_selected = None;
        }
        self.remote_selected_names
            .retain(|name| self.remote.entries.iter().any(|entry| &entry.name == name));
        if first_caps {
            self.request_remote_list(
                self.preferred_remote
                    .clone()
                    .unwrap_or_else(|| self.remote.remote_path.clone()),
            );
        } else if management_finished {
            self.refresh_remote();
        }
        if download_finished {
            self.reload_local(cx);
        }
        self.advance_queue(cx);
        cx.notify();
    }

    fn reconcile_photo_thumbs(&mut self, cx: &mut Context<Self>) {
        let visible: HashSet<&str> = self
            .remote
            .photo_entries
            .iter()
            .map(|entry| entry.id.as_str())
            .collect();
        let removed: Vec<_> = self
            .photo_thumbs
            .keys()
            .filter(|id| !visible.contains(id.as_str()))
            .cloned()
            .collect();
        for id in removed {
            if let Some(image) = self.photo_thumbs.remove(&id) {
                cx.drop_image(image, None);
            }
        }
        for entry in &self.remote.photo_entries {
            if let Ok(decoded) = image::load_from_memory(&entry.thumbnail) {
                let mut pixels = decoded.to_rgba8();
                // GPUI RenderImage textures use BGRA, while image decoders
                // return RGBA. Preserve green/alpha and swap red/blue once.
                for pixel in pixels.pixels_mut() {
                    pixel.0.swap(0, 2);
                }
                let frame = image::Frame::new(pixels);
                let image = Arc::new(RenderImage::new(SmallVec::from_elem(frame, 1)));
                if let Some(previous) = self.photo_thumbs.insert(entry.id.clone(), image) {
                    cx.drop_image(previous, None);
                }
            }
        }
    }

    fn refresh_remote(&self) {
        self.request_remote_list(self.remote.remote_path.clone());
    }

    fn request_remote_list(&self, path: String) {
        if let Some(client) = &self.client {
            client.send(FileCommand::List(path));
        }
    }

    fn request_photo_list(&self) {
        if let Some(client) = &self.client {
            client.send(FileCommand::PhotoList {
                offset: 0,
                album: self.remote.photo_album.clone(),
            });
        }
    }

    fn sync_photo_mode(&mut self, cx: &mut Context<Self>) {
        let show_photos = self.memory.folders.borrow().photo_mode && self.remote.can_photos;
        if self.photo_mode != show_photos {
            self.photo_mode = show_photos;
            if show_photos {
                self.request_photo_list();
            }
            cx.notify();
        }
    }

    fn persist_folders(&self, cx: &mut Context<Self>) {
        if let Some(id) = self.memory.connection_id
            && let Some(book) = self
                .memory
                .address_book
                .as_ref()
                .and_then(WeakEntity::upgrade)
        {
            let folders = self.memory.folders.borrow().clone();
            book.update(cx, |book, cx| {
                book.remember_transfer_folders(id, folders, cx)
            });
        }
    }

    fn navigate_remote(
        &mut self,
        path: String,
        remember: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.current_job.is_some()
            || self
                .queue
                .iter()
                .any(|job| matches!(job.state, QueueState::Queued))
        {
            self.status_error =
                Some("Finish the transfer queue before changing the remote folder".into());
            cx.notify();
            return;
        }
        let path = if path.len() > 1 {
            path.trim_end_matches('/').to_owned()
        } else {
            path
        };
        if !path.starts_with('/')
            || path.contains('\0')
            || path.split('/').any(|part| part == "." || part == "..")
        {
            self.status_error = Some("Enter an absolute remote folder path".into());
            cx.notify();
            return;
        }
        if remember && path != self.remote.remote_path {
            self.remote_back.push(self.remote.remote_path.clone());
        }
        if self.memory.folders.borrow().remote.as_deref() != Some(path.as_str()) {
            self.memory.folders.borrow_mut().remote = Some(path.clone());
            self.persist_folders(cx);
        }
        self.preferred_remote = Some(path.clone());
        self.status_error = None;
        self.remote_selected = None;
        self.remote_selected_names.clear();
        self.remote_anchor = None;
        self.remote_path_input
            .update(cx, |input, cx| input.set_value(path.clone(), window, cx));
        if let Some(client) = &self.client {
            client.send(FileCommand::List(path));
        }
        cx.notify();
    }

    fn go_remote(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = self.remote_path_input.read(cx).value().trim().to_owned();
        self.navigate_remote(path, true, window, cx);
    }

    fn upload(&mut self, cx: &mut Context<Self>) {
        let paths = if self.local_selected_paths.is_empty() {
            self.local_selected.clone().into_iter().collect()
        } else {
            self.local_selected_paths.clone()
        };
        let jobs = paths
            .into_iter()
            .filter(|path: &PathBuf| path.is_file())
            .filter_map(|source| {
                let name = source.file_name()?.to_str()?.to_owned();
                Some(QueueJob::new(
                    name.clone(),
                    QueueAction::Upload {
                        source,
                        remote: join_remote(&self.remote.remote_path, &name),
                        replace: None,
                    },
                ))
            })
            .collect();
        self.prepare_queue(jobs, cx);
    }

    fn upload_to_photos(&mut self, cx: &mut Context<Self>) {
        let paths = if self.local_selected_paths.is_empty() {
            self.local_selected.clone().into_iter().collect()
        } else {
            self.local_selected_paths.clone()
        };
        let selected = paths.len();
        let jobs: Vec<_> = paths
            .into_iter()
            .filter(|path| path.is_file() && is_image_name(&path.to_string_lossy()))
            .filter_map(|source| {
                let name = source.file_name()?.to_string_lossy().into_owned();
                Some(QueueJob::new(name, QueueAction::PhotoUpload(source)))
            })
            .collect();
        if jobs.len() < selected {
            self.status_error = Some(format!(
                "Skipped {} non-image or unavailable items",
                selected - jobs.len()
            ));
        }
        self.prepare_queue(jobs, cx);
    }

    fn import_remote_photo(&mut self, path: String, cx: &mut Context<Self>) {
        if let Some(client) = &self.client {
            client.send(FileCommand::PhotoImport {
                remote: path,
                expected_sha256: None,
            });
            cx.notify();
        }
    }

    fn download_photo(&mut self, cx: &mut Context<Self>) {
        let mut occupied: HashSet<String> = self
            .queue
            .iter()
            .filter_map(|job| match &job.action {
                QueueAction::Download { destination, .. }
                | QueueAction::PhotoExport { destination, .. } => {
                    destination.file_name()?.to_str().map(str::to_owned)
                }
                _ => None,
            })
            .collect();
        let jobs = self
            .remote
            .photo_entries
            .iter()
            .filter(|entry| self.photo_selected_ids.contains(&entry.id))
            .map(|entry| {
                let filename = Path::new(&entry.name)
                    .file_name()
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| std::ffi::OsStr::new("photo.jpg"));
                let name = filename.to_string_lossy().into_owned();
                let destination = photo_staging_path(&self.local_dir, &mut occupied);
                QueueJob::new(
                    name.clone(),
                    QueueAction::PhotoExport {
                        asset_id: entry.id.clone(),
                        destination,
                        replace: Some(self.local_dir.join(name)),
                    },
                )
            })
            .collect();
        self.prepare_queue(jobs, cx);
    }

    fn show_photos(&mut self, cx: &mut Context<Self>) {
        if self.remote.can_photos && !self.photo_mode {
            self.photo_mode = true;
            self.memory.folders.borrow_mut().photo_mode = true;
            self.persist_folders(cx);
            self.request_photo_list();
            cx.notify();
        }
    }

    fn show_files(&mut self, cx: &mut Context<Self>) {
        if self.photo_mode {
            self.photo_mode = false;
            self.memory.folders.borrow_mut().photo_mode = false;
            self.persist_folders(cx);
            self.refresh_remote();
            cx.notify();
        }
    }

    fn select_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.local_path_input.focus_handle(cx).is_focused(window)
            || self.remote_path_input.focus_handle(cx).is_focused(window)
            || self.new_folder_input.focus_handle(cx).is_focused(window)
        {
            return;
        }
        match self.active_side {
            FileSide::Local => {
                self.local_selected_paths = self
                    .local_entries
                    .iter()
                    .map(|entry| entry.path.clone())
                    .collect();
                self.local_selected = self.local_selected_paths.last().cloned();
                self.local_anchor = (!self.local_entries.is_empty()).then_some(0);
            }
            FileSide::Remote if self.photo_mode => {
                self.photo_selected_ids = self
                    .remote
                    .photo_entries
                    .iter()
                    .take(PHOTO_PAGE_SIZE)
                    .map(|entry| entry.id.clone())
                    .collect();
                self.photo_selected = self.photo_selected_ids.last().cloned();
                self.photo_anchor =
                    (!self.photo_selected_ids.is_empty()).then_some((self.remote.photo_offset, 0));
            }
            FileSide::Remote => {
                self.remote_selected_names = self
                    .remote
                    .entries
                    .iter()
                    .map(|entry| entry.name.clone())
                    .collect();
                self.remote_selected = self.remote_selected_names.last().cloned();
                self.remote_anchor = (!self.remote_selected_names.is_empty()).then_some(0);
            }
        }
        cx.notify();
    }

    fn download(&mut self, cx: &mut Context<Self>) {
        let names = if self.remote_selected_names.is_empty() {
            self.remote_selected.clone().into_iter().collect()
        } else {
            self.remote_selected_names.clone()
        };
        let jobs = names
            .into_iter()
            .filter_map(|name| {
                let entry = self
                    .remote
                    .entries
                    .iter()
                    .find(|entry| entry.name == name && !entry.is_dir)?;
                Some(QueueJob::new(
                    name.clone(),
                    QueueAction::Download {
                        remote: join_remote(&self.remote.remote_path, &name),
                        destination: self.local_dir.join(&name),
                        size: entry.size as u64,
                        replace: None,
                    },
                ))
            })
            .collect();
        self.prepare_queue(jobs, cx);
    }

    fn reserved_remote_names(&self) -> HashSet<String> {
        self.queue
            .iter()
            .filter(|job| {
                matches!(
                    job.state,
                    QueueState::Queued | QueueState::Running | QueueState::Complete
                )
            })
            .filter_map(|job| match &job.action {
                QueueAction::Upload {
                    remote, replace, ..
                } => Some(
                    replace
                        .as_deref()
                        .unwrap_or(remote)
                        .rsplit('/')
                        .next()?
                        .to_owned(),
                ),
                _ => None,
            })
            .collect()
    }

    fn reserved_local_names(&self) -> HashSet<String> {
        self.queue
            .iter()
            .filter(|job| {
                matches!(
                    job.state,
                    QueueState::Queued | QueueState::Running | QueueState::Complete
                )
            })
            .filter_map(|job| match &job.action {
                QueueAction::Download {
                    destination,
                    replace,
                    ..
                }
                | QueueAction::PhotoExport {
                    destination,
                    replace,
                    ..
                } => replace
                    .as_ref()
                    .unwrap_or(destination)
                    .file_name()?
                    .to_str()
                    .map(str::to_owned),
                _ => None,
            })
            .collect()
    }

    fn prepare_queue(&mut self, jobs: Vec<QueueJob>, cx: &mut Context<Self>) {
        if jobs.is_empty() {
            return;
        }
        let remote_reserved = self.reserved_remote_names();
        let local_reserved = self.reserved_local_names();
        let mut destinations = HashSet::new();
        let collision = jobs.iter().any(|job| match &job.action {
            QueueAction::Upload { remote, .. } => {
                self.remote
                    .entries
                    .iter()
                    .any(|entry| remote.ends_with(&format!("/{}", entry.name)))
                    || remote
                        .rsplit('/')
                        .next()
                        .is_some_and(|name| remote_reserved.contains(name))
            }
            QueueAction::Download { destination, .. } => {
                !destinations.insert(destination.clone())
                    || destination.exists()
                    || destination
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| local_reserved.contains(name))
            }
            QueueAction::PhotoExport { .. } => false,
            _ => false,
        });
        if collision {
            self.pending_conflicts = Some(jobs);
        } else {
            self.queue.extend(jobs);
            self.transfers_open = true;
            self.auto_collapse_armed = true;
            self.queue_idle_since = None;
            self.advance_queue(cx);
        }
        cx.notify();
    }

    fn resolve_conflicts(&mut self, policy: CollisionPolicy, cx: &mut Context<Self>) {
        let Some(jobs) = self.pending_conflicts.take() else {
            return;
        };
        let mut remote_names: HashSet<String> = self
            .remote
            .entries
            .iter()
            .map(|entry| entry.name.clone())
            .collect();
        remote_names.extend(self.reserved_remote_names());
        let mut local_names: HashSet<String> = self
            .local_entries
            .iter()
            .map(|entry| entry.name.clone())
            .collect();
        local_names.extend(self.reserved_local_names());
        for mut job in jobs {
            match &mut job.action {
                QueueAction::Upload {
                    remote, replace, ..
                } => {
                    let name = remote.rsplit('/').next().unwrap_or("").to_owned();
                    if remote_names.contains(&name) {
                        match policy {
                            CollisionPolicy::Skip => {
                                job.state = QueueState::Skipped;
                                self.queue.push(job);
                                continue;
                            }
                            CollisionPolicy::Rename => {
                                let unique = unique_copy_name(&name, &remote_names);
                                *remote = join_remote(&self.remote.remote_path, &unique);
                                job.name = unique.clone();
                                remote_names.insert(unique);
                            }
                            CollisionPolicy::Replace => {
                                if !self.remote.can_replace {
                                    self.status_error = Some(
                                        "This TrollVNC version cannot replace remote files".into(),
                                    );
                                    continue;
                                }
                                if self
                                    .remote
                                    .entries
                                    .iter()
                                    .any(|entry| entry.name == name && entry.is_dir)
                                {
                                    job.state = QueueState::Failed(
                                        "Cannot replace a remote folder with a file".into(),
                                    );
                                    self.queue.push(job);
                                    continue;
                                }
                                *replace = Some(remote.clone());
                                let unique =
                                    unique_copy_name(&format!(".rv-{name}"), &remote_names);
                                *remote = join_remote(&self.remote.remote_path, &unique);
                                remote_names.insert(unique);
                            }
                        }
                    } else {
                        remote_names.insert(name);
                    }
                }
                QueueAction::Download {
                    destination,
                    replace,
                    ..
                } => {
                    let name = destination
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    if local_names.contains(&name) || destination.exists() {
                        match policy {
                            CollisionPolicy::Skip => {
                                job.state = QueueState::Skipped;
                                self.queue.push(job);
                                continue;
                            }
                            CollisionPolicy::Rename => {
                                let mut unique = unique_copy_name(&name, &local_names);
                                while self.local_dir.join(&unique).exists() {
                                    local_names.insert(unique.clone());
                                    unique = unique_copy_name(&name, &local_names);
                                }
                                *destination = self.local_dir.join(&unique);
                                job.name = unique.clone();
                                local_names.insert(unique);
                            }
                            CollisionPolicy::Replace => {
                                if destination.is_dir() {
                                    job.state = QueueState::Failed(
                                        "Cannot replace a local folder with a file".into(),
                                    );
                                    self.queue.push(job);
                                    continue;
                                }
                                *replace = Some(destination.clone());
                                let temp_name = format!(".rv-{name}");
                                let mut unique = unique_copy_name(&temp_name, &local_names);
                                while self.local_dir.join(&unique).exists() {
                                    local_names.insert(unique.clone());
                                    unique = unique_copy_name(&temp_name, &local_names);
                                }
                                *destination = self.local_dir.join(&unique);
                                local_names.insert(unique);
                            }
                        }
                    } else {
                        local_names.insert(name);
                    }
                }
                _ => {}
            }
            self.queue.push(job);
        }
        self.transfers_open = true;
        self.auto_collapse_armed = true;
        self.queue_idle_since = None;
        self.advance_queue(cx);
        cx.notify();
    }

    fn advance_queue(&mut self, cx: &mut Context<Self>) {
        let mut reload_local = false;
        if let Some(index) = self.current_job {
            let job = &mut self.queue[index];
            match &job.action {
                QueueAction::Upload {
                    remote, replace, ..
                } => {
                    if job.replace_sent {
                        if self.remote.management_revision != job.management_revision {
                            job.state = QueueState::Complete;
                            self.current_job = None;
                        } else if !self.remote.management_busy
                            && self.remote.revision > job.started_revision
                            && let Some(error) = &self.remote.error
                        {
                            job.state = QueueState::Failed(error.clone());
                            self.current_job = None;
                        }
                    } else if let Some(record) = job
                        .transfer_index
                        .and_then(|i| self.remote.transfers.get(i))
                    {
                        match &record.status {
                            TransferStatus::Complete | TransferStatus::Verified => {
                                if let Some(target) = replace {
                                    if let Some(client) = &self.client {
                                        job.replace_sent = true;
                                        job.management_revision = self.remote.management_revision;
                                        job.started_revision = self.remote.revision;
                                        client.send(FileCommand::Replace {
                                            from: remote.clone(),
                                            to: target.clone(),
                                        });
                                    }
                                } else {
                                    job.state = QueueState::Complete;
                                    self.current_job = None;
                                }
                            }
                            TransferStatus::Failed(reason) => {
                                job.state = QueueState::Failed(reason.clone());
                                self.current_job = None;
                            }
                            TransferStatus::Cancelled => {
                                job.state = QueueState::Cancelled;
                                self.current_job = None;
                            }
                            _ => {}
                        }
                    } else if self.remote.revision > job.started_revision
                        && let Some(error) = &self.remote.error
                    {
                        job.state = QueueState::Failed(error.clone());
                        self.current_job = None;
                    }
                }
                QueueAction::Download {
                    destination,
                    replace,
                    ..
                } => {
                    if let Some(record) = job
                        .transfer_index
                        .and_then(|i| self.remote.transfers.get(i))
                    {
                        match &record.status {
                            TransferStatus::Complete | TransferStatus::Verified => {
                                if let Some(target) = replace {
                                    if let Err(error) = std::fs::rename(destination, target) {
                                        job.state = QueueState::Failed(format!(
                                            "Cannot replace local file: {error}"
                                        ));
                                    } else {
                                        job.state = QueueState::Complete;
                                        reload_local = true;
                                    }
                                } else {
                                    job.state = QueueState::Complete;
                                }
                                self.current_job = None;
                            }
                            TransferStatus::Failed(reason) => {
                                job.state = QueueState::Failed(reason.clone());
                                self.current_job = None;
                            }
                            TransferStatus::Cancelled => {
                                job.state = QueueState::Cancelled;
                                self.current_job = None;
                            }
                            _ => {}
                        }
                    } else if self.remote.revision > job.started_revision
                        && let Some(error) = &self.remote.error
                    {
                        job.state = QueueState::Failed(error.clone());
                        self.current_job = None;
                    }
                }
                QueueAction::PhotoExport {
                    destination,
                    replace,
                    ..
                } => {
                    if self.remote.photo_revision != job.photo_revision && !self.remote.photo_busy {
                        if let Some(error) = &self.remote.photo_error {
                            job.state = QueueState::Failed(error.clone());
                            let _ = std::fs::remove_file(destination);
                        } else if self.remote.photo_status.as_deref()
                            == Some("Original photo downloaded")
                        {
                            if let Some(target) = replace {
                                if let Err(error) = std::fs::rename(destination, target) {
                                    job.state = QueueState::Failed(format!(
                                        "Cannot replace local file: {error}"
                                    ));
                                } else {
                                    job.state = QueueState::Complete;
                                    reload_local = true;
                                }
                            } else {
                                job.state = QueueState::Complete;
                                reload_local = true;
                            }
                        } else {
                            job.state =
                                QueueState::Failed("Photo download did not complete".into());
                            let _ = std::fs::remove_file(destination);
                        }
                        self.current_job = None;
                    }
                }
                QueueAction::PhotoUpload(_) => {
                    if self.remote.photo_revision != job.photo_revision && !self.remote.photo_busy {
                        if self
                            .remote
                            .photo_status
                            .as_deref()
                            .is_some_and(|status| status.starts_with("Imported into Photos"))
                        {
                            job.state = QueueState::Complete;
                        } else if let Some(error) = &self.remote.photo_error {
                            job.state = QueueState::Failed(error.clone());
                        } else {
                            job.state = QueueState::Failed("Photo upload did not complete".into());
                        }
                        self.current_job = None;
                    }
                }
                QueueAction::DeleteRemote(_) => {
                    if self.remote.management_revision != job.management_revision {
                        job.state = QueueState::Complete;
                        self.current_job = None;
                    } else if !self.remote.management_busy
                        && self.remote.revision > job.started_revision
                        && let Some(error) = &self.remote.error
                    {
                        job.state = QueueState::Failed(error.clone());
                        self.current_job = None;
                    }
                }
                QueueAction::DeleteLocal(_) => {
                    self.current_job = None;
                }
            }
            if self.current_job.is_some()
                && job.transfer_index.is_none()
                && !matches!(
                    job.action,
                    QueueAction::DeleteRemote(_) | QueueAction::PhotoExport { .. }
                )
                && let Some(record) = self.remote.transfers.get(job.transfer_start)
            {
                let direction_ok = matches!(
                    (&job.action, record.direction),
                    (QueueAction::Upload { .. }, TransferDirection::Upload)
                        | (QueueAction::PhotoUpload(_), TransferDirection::Upload)
                        | (QueueAction::Download { .. }, TransferDirection::Download)
                );
                if direction_ok {
                    job.transfer_index = Some(job.transfer_start);
                }
            }
        }
        if reload_local {
            self.reload_local(cx);
        }
        if self.current_job.is_some() {
            return;
        }
        if self.remote.listed_path.as_deref() == Some(self.remote.remote_path.as_str()) {
            let mut cleanup = Vec::new();
            for job in &mut self.queue {
                if job.cleanup_enqueued
                    || !matches!(job.state, QueueState::Failed(_) | QueueState::Cancelled)
                {
                    continue;
                }
                if let QueueAction::Upload {
                    remote,
                    replace: Some(_),
                    ..
                } = &job.action
                {
                    let name = remote.rsplit('/').next().unwrap_or("");
                    if self.remote.entries.iter().any(|entry| entry.name == name) {
                        job.cleanup_enqueued = true;
                        cleanup.push(QueueJob::new(
                            format!("Remove temporary {name}"),
                            QueueAction::DeleteRemote(remote.clone()),
                        ));
                    }
                }
            }
            let at = self
                .queue
                .iter()
                .position(|job| matches!(job.state, QueueState::Queued))
                .unwrap_or(self.queue.len());
            for (offset, job) in cleanup.into_iter().enumerate() {
                self.queue.insert(at + offset, job);
            }
        }
        let Some(index) = self
            .queue
            .iter()
            .position(|job| matches!(job.state, QueueState::Queued))
        else {
            if self.remote.photos_need_refresh && !self.remote.photo_busy {
                // Includes failed/cancelled batches: refresh successfully imported
                // items even when the last queued upload did not complete.
                self.remote.photo_busy = true;
                self.remote.photos_need_refresh = false;
                if let Some(client) = &self.client {
                    client.send(FileCommand::PhotoList {
                        offset: self.remote.photo_offset,
                        album: self.remote.photo_album.clone(),
                    });
                }
            }
            if self.auto_collapse_armed
                && self
                    .queue
                    .iter()
                    .all(|job| matches!(job.state, QueueState::Complete | QueueState::Skipped))
            {
                let since = self.queue_idle_since.get_or_insert_with(Instant::now);
                if since.elapsed() >= Duration::from_secs(4) {
                    self.transfers_open = false;
                    self.auto_collapse_armed = false;
                    cx.notify();
                }
            }
            return;
        };
        if matches!(
            self.queue[index].action,
            QueueAction::Upload { .. } | QueueAction::DeleteRemote(_)
        ) && self.remote.listed_path.as_deref() != Some(self.remote.remote_path.as_str())
        {
            return;
        }
        if matches!(
            self.queue[index].action,
            QueueAction::PhotoExport { .. } | QueueAction::PhotoUpload(_)
        ) && (self.remote.photo_busy
            || self.remote.transfers.iter().any(|transfer| {
                matches!(
                    transfer.status,
                    TransferStatus::Running | TransferStatus::Sent
                )
            }))
        {
            return;
        }
        self.queue_idle_since = None;
        let job = &mut self.queue[index];
        job.state = QueueState::Running;
        job.transfer_index = None;
        job.started_revision = self.remote.revision;
        job.management_revision = self.remote.management_revision;
        job.photo_revision = self.remote.photo_revision;
        self.current_job = Some(index);
        match &job.action {
            QueueAction::Upload { source, remote, .. } => {
                job.transfer_start = self.remote.transfers.len();
                if let Some(client) = &self.client {
                    client.send(FileCommand::Upload {
                        source: source.clone(),
                        remote: remote.clone(),
                    });
                }
            }
            QueueAction::Download {
                remote,
                destination,
                size,
                ..
            } => {
                job.transfer_start = self.remote.transfers.len();
                if let Some(client) = &self.client {
                    client.send(FileCommand::Download {
                        remote: remote.clone(),
                        destination: destination.clone(),
                        size: *size,
                    });
                }
            }
            QueueAction::PhotoExport {
                asset_id,
                destination,
                ..
            } => {
                if let Some(client) = &self.client {
                    client.send(FileCommand::PhotoExport {
                        asset_id: asset_id.clone(),
                        destination: destination.clone(),
                    });
                }
            }
            QueueAction::PhotoUpload(source) => {
                job.transfer_start = self.remote.transfers.len();
                if let Some(client) = &self.client {
                    client.send(FileCommand::UploadToPhotosQueued(source.clone()));
                }
            }
            QueueAction::DeleteRemote(path) => {
                if let Some(client) = &self.client {
                    client.send(FileCommand::Delete(path.clone()));
                }
            }
            QueueAction::DeleteLocal(path) => {
                let result = std::fs::symlink_metadata(path).and_then(|meta| {
                    if meta.is_dir() {
                        std::fs::remove_dir(path)
                    } else {
                        std::fs::remove_file(path)
                    }
                });
                job.state = match result {
                    Ok(()) => QueueState::Complete,
                    Err(error) => QueueState::Failed(error.to_string()),
                };
                self.current_job = None;
                self.reload_local(cx);
            }
        }
        cx.notify();
    }

    fn confirm_delete(&mut self, cx: &mut Context<Self>) {
        let Some(details) = self.pending_delete.take() else {
            return;
        };
        self.status_error = None;
        match details.side {
            FileSide::Local => {
                let path = PathBuf::from(details.path);
                let result = std::fs::symlink_metadata(&path).and_then(|metadata| {
                    if metadata.is_dir() {
                        std::fs::remove_dir(&path)
                    } else {
                        std::fs::remove_file(&path)
                    }
                });
                if let Err(error) = result {
                    self.status_error = Some(format!("Cannot delete local item: {error}"));
                } else {
                    self.local_selected = None;
                    self.reload_local(cx);
                }
            }
            FileSide::Remote => {
                if self.allow_upload
                    && self.remote.can_delete
                    && !self.remote.management_busy
                    && let Some(client) = &self.client
                {
                    client.send(FileCommand::Delete(details.path));
                }
            }
        }
        cx.notify();
    }

    fn confirm_batch_delete(&mut self, cx: &mut Context<Self>) {
        let Some(side) = self.pending_batch_delete.take() else {
            return;
        };
        let jobs = match side {
            FileSide::Local => self
                .local_selected_paths
                .iter()
                .map(|path| {
                    QueueJob::new(
                        path.file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned(),
                        QueueAction::DeleteLocal(path.clone()),
                    )
                })
                .collect::<Vec<_>>(),
            FileSide::Remote => self
                .remote_selected_names
                .iter()
                .map(|name| {
                    QueueJob::new(
                        name.clone(),
                        QueueAction::DeleteRemote(join_remote(&self.remote.remote_path, name)),
                    )
                })
                .collect::<Vec<_>>(),
        };
        self.queue.extend(jobs);
        self.transfers_open = true;
        self.auto_collapse_armed = true;
        self.queue_idle_since = None;
        self.advance_queue(cx);
        cx.notify();
    }

    fn retry_job(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(job) = self.queue.get(index).cloned() else {
            return;
        };
        if !matches!(job.state, QueueState::Failed(_) | QueueState::Cancelled) {
            return;
        }
        let replace_failed = matches!(&job.state, QueueState::Failed(reason) if reason.starts_with("Cannot replace local file:"));
        let mut retry = QueueJob::new(job.name, job.action);
        if let QueueAction::Download {
            destination,
            replace,
            ..
        } = &mut retry.action
            && let Some(target) = replace.clone()
        {
            if destination.exists() {
                match std::fs::rename(&*destination, &target) {
                    Ok(()) => {
                        self.queue[index].state = QueueState::Complete;
                        self.reload_local(cx);
                        cx.notify();
                    }
                    Err(error) => {
                        self.status_error = Some(format!("Cannot replace local file: {error}"));
                        cx.notify();
                    }
                }
                return;
            }
            *destination = target;
            *replace = None;
        }
        if let QueueAction::PhotoExport {
            destination,
            replace: Some(target),
            ..
        } = &mut retry.action
        {
            if replace_failed && destination.exists() {
                match std::fs::rename(&*destination, &*target) {
                    Ok(()) => {
                        self.queue[index].state = QueueState::Complete;
                        self.reload_local(cx);
                    }
                    Err(error) => {
                        self.status_error = Some(format!("Cannot replace local file: {error}"));
                    }
                }
                cx.notify();
                return;
            }
            let _ = std::fs::remove_file(&*destination);
            let mut occupied: HashSet<String> = self
                .queue
                .iter()
                .filter_map(|job| match &job.action {
                    QueueAction::Download { destination, .. }
                    | QueueAction::PhotoExport { destination, .. } => {
                        destination.file_name()?.to_str().map(str::to_owned)
                    }
                    _ => None,
                })
                .collect();
            *destination = photo_staging_path(&self.local_dir, &mut occupied);
        }
        if let QueueAction::Upload {
            remote,
            replace: Some(_),
            ..
        } = &mut retry.action
        {
            let occupied: HashSet<String> = self
                .remote
                .entries
                .iter()
                .map(|entry| entry.name.clone())
                .collect();
            let name = remote.rsplit('/').next().unwrap_or(".rv-upload");
            *remote = join_remote(&self.remote.remote_path, &unique_copy_name(name, &occupied));
        }
        self.prepare_queue(vec![retry], cx);
    }

    fn create_folder(&mut self, cx: &mut Context<Self>) {
        let Some(side) = self.new_folder_side.take() else {
            return;
        };
        self.status_error = None;
        let name = self.new_folder_input.read(cx).value().trim().to_owned();
        if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0']) {
            self.status_error = Some("Enter a single folder name".into());
            cx.notify();
            return;
        }
        match side {
            FileSide::Local => {
                if let Err(error) = std::fs::create_dir(self.local_dir.join(&name)) {
                    self.status_error = Some(format!("Cannot create local folder: {error}"));
                } else {
                    self.reload_local(cx);
                }
            }
            FileSide::Remote => {
                if let Some(client) = &self.client {
                    client.send(FileCommand::CreateFolder(join_remote(
                        &self.remote.remote_path,
                        &name,
                    )));
                }
            }
        }
        cx.notify();
    }

    fn rename_item(&mut self, cx: &mut Context<Self>) {
        let Some(details) = self.pending_rename.take() else {
            return;
        };
        self.status_error = None;
        let name = self.new_folder_input.read(cx).value().trim().to_owned();
        if name.is_empty()
            || name == "."
            || name == ".."
            || name.contains(['/', '\\', '\0'])
            || name == details.name
        {
            self.status_error = Some("Enter a different single file name".into());
            cx.notify();
            return;
        }
        match details.side {
            FileSide::Local => {
                let from = PathBuf::from(details.path);
                let to = from.with_file_name(name);
                if let Err(error) = rename_local_no_replace(&from, &to) {
                    self.status_error = Some(format!("Cannot rename local item: {error}"));
                } else {
                    self.local_selected = Some(to);
                    self.reload_local(cx);
                }
            }
            FileSide::Remote => {
                if self.allow_upload
                    && self.remote.can_rename
                    && !self.remote.management_busy
                    && let Some(client) = &self.client
                {
                    client.send(FileCommand::Rename {
                        from: details.path,
                        to: join_remote(&self.remote.remote_path, &name),
                    });
                }
            }
        }
        cx.notify();
    }

    fn reload_local(&mut self, cx: &mut Context<Self>) {
        let path = self.local_dir.clone();
        let read_path = path.clone();
        self.local_loading = true;
        self.local_error = None;
        self.local_entries.clear();
        let load = cx
            .background_executor()
            .spawn(async move { read_local_dir(&read_path) });
        cx.spawn(async move |this, cx| {
            let result = load.await;
            let _ = this.update(cx, |this, cx| {
                if this.local_dir != path {
                    return;
                }
                this.local_loading = false;
                match result {
                    Ok(entries) => {
                        this.local_entries = entries;
                        this.sort_local_entries();
                        this.local_selected_paths.retain(|path| {
                            this.local_entries.iter().any(|entry| &entry.path == path)
                        });
                    }
                    Err(error) => {
                        this.local_entries.clear();
                        this.local_error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn navigate_local(
        &mut self,
        path: PathBuf,
        remember: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !path.is_dir() {
            self.status_error = Some(format!("Local folder is unavailable: {}", path.display()));
            cx.notify();
            return;
        }
        if remember && path != self.local_dir {
            self.local_back.push(self.local_dir.clone());
        }
        self.status_error = None;
        self.local_dir = path;
        if self.memory.folders.borrow().local.as_ref() != Some(&self.local_dir) {
            self.memory.folders.borrow_mut().local = Some(self.local_dir.clone());
            self.persist_folders(cx);
        }
        self.local_selected = None;
        self.local_selected_paths.clear();
        self.local_anchor = None;
        self.local_path_input.update(cx, |input, cx| {
            input.set_value(self.local_dir.display().to_string(), window, cx)
        });
        self.reload_local(cx);
        cx.notify();
    }

    fn go_local(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.local_path_input.read(cx).value().trim().to_owned();
        let path = if text == "~" {
            UserDirs::new().map(|user| user.home_dir().to_path_buf())
        } else if let Some(rest) = text.strip_prefix("~/") {
            UserDirs::new().map(|user| user.home_dir().join(rest))
        } else {
            let path = PathBuf::from(text);
            Some(if path.is_absolute() {
                path
            } else {
                self.local_dir.join(path)
            })
        };
        if let Some(path) = path {
            self.navigate_local(path, true, window, cx);
        }
    }

    fn sort_local_entries(&mut self) {
        let sort = self.local_sort;
        self.local_entries.sort_by(|a, b| {
            let order = match sort.column {
                SortColumn::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                SortColumn::Size => a.size_bytes.cmp(&b.size_bytes),
                SortColumn::Modified => a.modified_time.cmp(&b.modified_time),
            };
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| {
                    if sort.ascending {
                        order
                    } else {
                        order.reverse()
                    }
                })
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
    }

    fn sort_remote_entries(&mut self) {
        let sort = self.remote_sort;
        self.remote.entries.sort_by(|a, b| {
            let order = match sort.column {
                SortColumn::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                SortColumn::Size => a.size.cmp(&b.size),
                SortColumn::Modified => a.modified.cmp(&b.modified),
            };
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| {
                    if sort.ascending {
                        order
                    } else {
                        order.reverse()
                    }
                })
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
    }

    fn change_sort(&mut self, side: FileSide, column: SortColumn, cx: &mut Context<Self>) {
        let sort = if side == FileSide::Local {
            &mut self.local_sort
        } else {
            &mut self.remote_sort
        };
        if sort.column == column {
            sort.ascending = !sort.ascending;
        } else {
            sort.column = column;
            sort.ascending = true;
        }
        match side {
            FileSide::Local => {
                self.sort_local_entries();
                self.local_anchor = None;
            }
            FileSide::Remote => {
                self.sort_remote_entries();
                self.remote_anchor = None;
            }
        }
        cx.notify();
    }

    fn render_local_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let entry = &self.local_entries[index];
        let path = entry.path.clone();
        let is_dir = entry.is_dir;
        let selected = self.local_selected_paths.contains(&path);
        let details = FileDetails {
            side: FileSide::Local,
            name: entry.name.clone(),
            path: path.display().to_string(),
            is_dir,
            size: entry.size_bytes,
            modified: entry.modified_time,
        };
        let right_path = path.clone();
        h_flex()
            .id(("local-file", index))
            .debug_selector(move || format!("local-file-{index}"))
            .w_full()
            .h(px(26.))
            .px_2()
            .gap_2()
            .items_center()
            .cursor_pointer()
            .bg(rgb(if selected {
                SELECTED
            } else if index % 2 == 1 {
                ROW_ALT
            } else {
                PANEL
            }))
            .text_xs()
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _, window, cx| {
                    this.active_side = FileSide::Local;
                    this.focus.focus(window, cx);
                    this.local_selected = Some(right_path.clone());
                    if !this.local_selected_paths.contains(&right_path) {
                        this.local_selected_paths = vec![right_path.clone()];
                    }
                    cx.notify();
                }),
            )
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.active_side = FileSide::Local;
                this.focus.focus(window, cx);
                let modifiers = event.modifiers();
                if is_dir && !modifiers.secondary() && !modifiers.shift {
                    this.navigate_local(path.clone(), true, window, cx);
                } else {
                    let additive = modifiers.secondary();
                    if modifiers.shift
                        && let Some(anchor) = this.local_anchor
                    {
                        if !additive {
                            this.local_selected_paths.clear();
                        }
                        for row in anchor.min(index)..=anchor.max(index) {
                            let path = this.local_entries[row].path.clone();
                            if !this.local_selected_paths.contains(&path) {
                                this.local_selected_paths.push(path);
                            }
                        }
                    } else if additive {
                        if let Some(pos) = this
                            .local_selected_paths
                            .iter()
                            .position(|item| item == &path)
                        {
                            this.local_selected_paths.remove(pos);
                        } else {
                            this.local_selected_paths.push(path.clone());
                        }
                        this.local_anchor = Some(index);
                    } else {
                        this.local_selected_paths = vec![path.clone()];
                        this.local_anchor = Some(index);
                    }
                    this.local_selected = this.local_selected_paths.last().cloned();
                    cx.notify();
                }
            }))
            .context_menu(file_menu(
                cx.entity(),
                details,
                self.allow_upload
                    && self.remote.caps.is_some_and(|caps| caps.upload)
                    && self.remote.listed_path.as_deref() == Some(self.remote.remote_path.as_str()),
                true,
                true,
                self.allow_upload
                    && self.remote.can_photos
                    && !self.remote.photo_busy
                    && !is_dir
                    && is_image_name(&entry.name),
            ))
            .child(
                Icon::new(if is_dir {
                    IconName::Folder
                } else {
                    IconName::File
                })
                .small()
                .text_color(rgb(if is_dir { 0x69b8ef } else { MUTED })),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_ellipsis()
                    .child(entry.name.clone()),
            )
            .child(
                div()
                    .w(px(70.))
                    .text_color(rgb(MUTED))
                    .child(entry.size.clone()),
            )
            .child(
                div()
                    .w(px(92.))
                    .text_color(rgb(MUTED))
                    .text_ellipsis()
                    .child(entry.modified.clone()),
            )
            .into_any_element()
    }

    fn render_local_pane(&self, cx: &mut Context<Self>) -> AnyElement {
        let parent = self.local_dir.parent().map(Path::to_path_buf);
        let back = self.local_back.last().cloned();
        v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .min_h_0()
            .bg(rgb(PANEL))
            .child(
                h_flex()
                    .h(px(28.))
                    .px_2()
                    .gap_2()
                    .items_center()
                    .font_semibold()
                    .text_sm()
                    .child(Icon::new(IconName::Folder).small())
                    .child("Local")
                    .child(div().flex_1())
                    .child(
                        Button::new("local-new-folder")
                            .xsmall()
                            .ghost()
                            .text_color(rgb(TEXT))
                            .icon(IconName::Plus)
                            .label("New folder")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.new_folder_side = Some(FileSide::Local);
                                this.new_folder_input.update(cx, |input, cx| {
                                    input.set_value("", window, cx);
                                    input.focus(window, cx);
                                });
                                cx.notify();
                            })),
                    ),
            )
            .child(
                h_flex()
                    .h(px(27.))
                    .px_2()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(LINE))
                    .child(
                        Button::new("local-back")
                            .xsmall()
                            .ghost()
                            .text_color(rgb(TEXT))
                            .icon(IconName::ChevronLeft)
                            .tooltip("Previous local folder")
                            .disabled(back.is_none())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if let Some(path) = this.local_back.pop() {
                                    this.navigate_local(path, false, window, cx);
                                }
                            })),
                    )
                    .child(
                        Button::new("local-parent")
                            .xsmall()
                            .ghost()
                            .text_color(rgb(TEXT))
                            .icon(IconName::ArrowUp)
                            .tooltip("Parent folder")
                            .disabled(parent.is_none())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if let Some(parent) = parent.clone() {
                                    this.navigate_local(parent, true, window, cx);
                                }
                            })),
                    )
                    .child(
                        Input::new(&self.local_path_input)
                            .xsmall()
                            .appearance(false)
                            .aria_label("Local folder path")
                            .flex_1()
                            .min_w_0(),
                    ),
            )
            .child(self.table_header(FileSide::Local, cx))
            .child(if self.local_loading {
                empty_message("Loading local files…").into_any_element()
            } else if let Some(error) = &self.local_error {
                empty_message(error.clone()).into_any_element()
            } else if self.local_entries.is_empty() {
                empty_message("This folder is empty").into_any_element()
            } else {
                uniform_list(
                    "local-files",
                    self.local_entries.len(),
                    cx.processor(|this, range: Range<usize>, _, cx| {
                        range
                            .map(|index| this.render_local_row(index, cx))
                            .collect::<Vec<_>>()
                    }),
                )
                .flex_1()
                .min_h_0()
                .into_any_element()
            })
            .into_any_element()
    }

    fn render_remote_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let entry = &self.remote.entries[index];
        let name = entry.name.clone();
        let is_dir = entry.is_dir;
        let selected = self.remote_selected_names.contains(&name);
        let path = join_remote(&self.remote.remote_path, &name);
        let details = FileDetails {
            side: FileSide::Remote,
            name: name.clone(),
            path: path.clone(),
            is_dir,
            size: (!is_dir).then_some(entry.size as u64),
            modified: (entry.modified != 0)
                .then_some(SystemTime::UNIX_EPOCH + Duration::from_secs(entry.modified as u64)),
        };
        let right_name = name.clone();
        h_flex()
            .id(("remote-file", index))
            .debug_selector(move || format!("remote-file-{index}"))
            .w_full()
            .h(px(26.))
            .px_2()
            .gap_2()
            .items_center()
            .cursor_pointer()
            .bg(rgb(if selected {
                SELECTED
            } else if index % 2 == 1 {
                ROW_ALT
            } else {
                PANEL
            }))
            .text_xs()
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _, window, cx| {
                    this.active_side = FileSide::Remote;
                    this.focus.focus(window, cx);
                    this.remote_selected = Some(right_name.clone());
                    if !this.remote_selected_names.contains(&right_name) {
                        this.remote_selected_names = vec![right_name.clone()];
                    }
                    cx.notify();
                }),
            )
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.active_side = FileSide::Remote;
                this.focus.focus(window, cx);
                let modifiers = event.modifiers();
                if is_dir && !modifiers.secondary() && !modifiers.shift {
                    this.navigate_remote(path.clone(), true, window, cx);
                } else {
                    let additive = modifiers.secondary();
                    if modifiers.shift
                        && let Some(anchor) = this.remote_anchor
                    {
                        if !additive {
                            this.remote_selected_names.clear();
                        }
                        for row in anchor.min(index)..=anchor.max(index) {
                            let name = this.remote.entries[row].name.clone();
                            if !this.remote_selected_names.contains(&name) {
                                this.remote_selected_names.push(name);
                            }
                        }
                    } else if additive {
                        if let Some(pos) = this
                            .remote_selected_names
                            .iter()
                            .position(|item| item == &name)
                        {
                            this.remote_selected_names.remove(pos);
                        } else {
                            this.remote_selected_names.push(name.clone());
                        }
                        this.remote_anchor = Some(index);
                    } else {
                        this.remote_selected_names = vec![name.clone()];
                        this.remote_anchor = Some(index);
                    }
                    this.remote_selected = this.remote_selected_names.last().cloned();
                    cx.notify();
                }
            }))
            .context_menu(file_menu(
                cx.entity(),
                details,
                self.remote.caps.is_some_and(|caps| caps.download) && !self.remote.download_blocked,
                self.allow_upload
                    && self.remote.can_delete
                    && !self.remote.management_busy
                    && self.remote.listed_path.as_deref() == Some(self.remote.remote_path.as_str()),
                self.allow_upload
                    && self.remote.can_rename
                    && !self.remote.management_busy
                    && self.remote.listed_path.as_deref() == Some(self.remote.remote_path.as_str()),
                self.allow_upload
                    && self.remote.can_photos
                    && !self.remote.photo_busy
                    && !is_dir
                    && is_image_name(&entry.name),
            ))
            .child(
                Icon::new(if is_dir {
                    IconName::Folder
                } else {
                    IconName::File
                })
                .small()
                .text_color(rgb(if is_dir { 0x69b8ef } else { MUTED })),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_ellipsis()
                    .child(entry.name.clone()),
            )
            .child(div().w(px(70.)).text_color(rgb(MUTED)).child(if is_dir {
                "—".into()
            } else {
                format_size(entry.size as u64)
            }))
            .child(
                div()
                    .w(px(92.))
                    .text_color(rgb(MUTED))
                    .child(if entry.modified == 0 {
                        "—".into()
                    } else {
                        relative_time(
                            SystemTime::UNIX_EPOCH + Duration::from_secs(entry.modified as u64),
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_photo_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let entry = &self.remote.photo_entries[index];
        let id = entry.id.clone();
        let right_id = id.clone();
        let selected = self.photo_selected_ids.contains(&id);
        let offset = self.remote.photo_offset;
        let can_multi = true;
        let thumbnail = self.photo_thumbs.get(&id).cloned();
        let date = if entry.created > 0 {
            let timestamp = SystemTime::UNIX_EPOCH + Duration::from_secs(entry.created as u64);
            DateTime::<Local>::from(timestamp)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        } else {
            "—".into()
        };
        h_flex()
            .id(("photo", index))
            .debug_selector(move || format!("photo-{index}"))
            .w_full()
            .h(px(52.))
            .px_2()
            .gap_2()
            .items_center()
            .cursor_pointer()
            .bg(rgb(if selected {
                SELECTED
            } else if index % 2 == 1 {
                ROW_ALT
            } else {
                PANEL
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _, window, cx| {
                    this.active_side = FileSide::Remote;
                    this.focus.focus(window, cx);
                    if !this.photo_selected_ids.contains(&right_id) {
                        this.photo_selected_ids = vec![right_id.clone()];
                    }
                    this.photo_selected = Some(right_id.clone());
                    cx.notify();
                }),
            )
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.active_side = FileSide::Remote;
                this.focus.focus(window, cx);
                let modifiers = event.modifiers();
                let additive = can_multi && modifiers.secondary();
                if can_multi
                    && modifiers.shift
                    && this.photo_anchor.is_some_and(|(page, _)| page == offset)
                {
                    let (_, anchor) = this.photo_anchor.unwrap();
                    if !additive {
                        this.photo_selected_ids.clear();
                    }
                    for row in anchor.min(index)..=anchor.max(index) {
                        if let Some(entry) = this.remote.photo_entries.get(row)
                            && !this.photo_selected_ids.contains(&entry.id)
                            && this.photo_selected_ids.len() < 50
                        {
                            this.photo_selected_ids.push(entry.id.clone());
                        }
                    }
                } else if additive {
                    if let Some(position) = this
                        .photo_selected_ids
                        .iter()
                        .position(|selected| selected == &id)
                    {
                        this.photo_selected_ids.remove(position);
                    } else if this.photo_selected_ids.len() < 50 {
                        this.photo_selected_ids.push(id.clone());
                    } else {
                        this.status_error = Some("Select at most 50 photos at a time".into());
                    }
                    this.photo_anchor = Some((offset, index));
                } else {
                    this.photo_selected_ids.clear();
                    this.photo_selected_ids.push(id.clone());
                    this.photo_anchor = Some((offset, index));
                }
                this.photo_selected = if this.photo_selected_ids.contains(&id) {
                    Some(id.clone())
                } else {
                    this.photo_selected_ids.last().cloned()
                };
                cx.notify();
            }))
            .context_menu(photo_menu(
                cx.entity(),
                entry.clone(),
                self.allow_upload && self.remote.can_photo_delete && !self.remote.photo_busy,
                self.remote.can_photo_batch_delete,
                self.photo_selected_ids.clone(),
            ))
            .child(
                div()
                    .w(px(42.))
                    .h(px(42.))
                    .flex_none()
                    .rounded_sm()
                    .overflow_hidden()
                    .bg(rgb(ROW_ALT))
                    .child(if let Some(thumbnail) = thumbnail {
                        img(thumbnail)
                            .size_full()
                            .object_fit(ObjectFit::Cover)
                            .into_any_element()
                    } else {
                        div()
                            .size_full()
                            .items_center()
                            .justify_center()
                            .child(Icon::new(IconName::File).small().text_color(rgb(MUTED)))
                            .into_any_element()
                    }),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(div().text_sm().text_ellipsis().child(entry.name.clone()))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .child(format!("{} · {}×{}", date, entry.width, entry.height)),
                    ),
            )
            .into_any_element()
    }

    fn render_photos_pane(&self, cx: &mut Context<Self>) -> AnyElement {
        let available = self.remote.can_photos;
        let previous = self.remote.photo_offset.saturating_sub(PHOTO_PAGE_SIZE);
        let next = self.remote.photo_offset + self.remote.photo_entries.len();
        v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .min_h_0()
            .bg(rgb(PANEL))
            .child(
                h_flex()
                    .h(px(28.))
                    .px_2()
                    .gap_2()
                    .items_center()
                    .font_semibold()
                    .text_sm()
                    .child(Icon::new(IconName::File).small())
                    .child("Remote")
                    .child(div().flex_1())
                    .child(
                        Button::new("photos-files")
                            .xsmall()
                            .ghost()
                            .text_color(rgb(MUTED))
                            .icon(IconName::Folder)
                            .label("Files")
                            .on_click(cx.listener(|this, _, _, cx| this.show_files(cx))),
                    )
                    .child(
                        Button::new("photos-photos")
                            .xsmall()
                            .ghost()
                            .text_color(rgb(TEXT))
                            .icon(IconName::GalleryVerticalEnd)
                            .label("Photos"),
                    ),
            )
            .child(
                h_flex()
                    .h(px(27.))
                    .px_2()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(LINE))
                    .text_xs()
                    .child(
                        Button::new("photo-albums")
                            .xsmall()
                            .ghost()
                            .text_color(rgb(TEXT))
                            .label(
                                self.remote
                                    .photo_album
                                    .as_ref()
                                    .and_then(|id| {
                                        self.remote
                                            .photo_albums
                                            .iter()
                                            .find(|album| &album.id == id)
                                            .map(|album| album.name.clone())
                                    })
                                    .unwrap_or_else(|| "All photos".into()),
                            )
                            .icon(IconName::ChevronDown)
                            .disabled(
                                !available
                                    || self.remote.photo_busy
                                    || self.remote.photo_albums.is_empty(),
                            )
                            .dropdown_menu(album_menu(
                                cx.entity(),
                                self.remote.photo_albums.clone(),
                            )),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("photo-prev")
                            .xsmall()
                            .ghost()
                            .text_color(rgb(TEXT))
                            .icon(IconName::ChevronLeft)
                            .disabled(
                                !available
                                    || self.remote.photo_busy
                                    || self.remote.photo_offset == 0,
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(client) = &this.client {
                                    client.send(FileCommand::PhotoList {
                                        offset: previous,
                                        album: this.remote.photo_album.clone(),
                                    });
                                }
                                cx.notify();
                            })),
                    )
                    .child(format!(
                        "{}–{} / {}",
                        if self.remote.photo_total == 0 {
                            0
                        } else {
                            self.remote.photo_offset + 1
                        },
                        next,
                        self.remote.photo_total
                    ))
                    .child(
                        Button::new("photo-next")
                            .xsmall()
                            .ghost()
                            .text_color(rgb(TEXT))
                            .icon(IconName::ChevronRight)
                            .disabled(
                                !available
                                    || self.remote.photo_busy
                                    || next >= self.remote.photo_total,
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(client) = &this.client {
                                    client.send(FileCommand::PhotoList {
                                        offset: next,
                                        album: this.remote.photo_album.clone(),
                                    });
                                }
                                cx.notify();
                            })),
                    ),
            )
            .child(if !available {
                empty_message("Photos is unavailable on this TrollVNC connection")
                    .into_any_element()
            } else if self.remote.photo_busy && self.remote.photo_entries.is_empty() {
                empty_message("Loading Photos…").into_any_element()
            } else if let Some(error) = &self.remote.photo_error {
                empty_message(error.clone()).into_any_element()
            } else if self.remote.photo_entries.is_empty() {
                empty_message("No photos in this view").into_any_element()
            } else {
                uniform_list(
                    "photos",
                    self.remote.photo_entries.len(),
                    cx.processor(|this, range: Range<usize>, _, cx| {
                        range
                            .map(|index| this.render_photo_row(index, cx))
                            .collect::<Vec<_>>()
                    }),
                )
                .flex_1()
                .min_h_0()
                .into_any_element()
            })
            .into_any_element()
    }

    fn render_remote_pane(&self, cx: &mut Context<Self>) -> AnyElement {
        let parent = remote_parent(&self.remote.remote_path);
        let back = self.remote_back.last().cloned();
        let available = self.remote.caps.is_some_and(|caps| caps.list);
        let message = if let Some(error) = &self.remote.error {
            error.clone()
        } else if self.remote.listing {
            "Loading remote files…".into()
        } else if self.remote.caps.is_none() {
            "Waiting for TightVNC file capabilities…".into()
        } else if !available {
            "File transfer is unavailable on this connection. Enable -T on in TrollVNC.".into()
        } else {
            "This folder is empty".into()
        };
        v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .min_h_0()
            .bg(rgb(PANEL))
            .child(
                h_flex()
                    .h(px(28.))
                    .px_2()
                    .gap_2()
                    .items_center()
                    .font_semibold()
                    .text_sm()
                    .child(Icon::new(IconName::Folder).small())
                    .child("Remote")
                    .child(div().flex_1())
                    .child(
                        Button::new("files-files")
                            .xsmall()
                            .ghost()
                            .text_color(rgb(TEXT))
                            .icon(IconName::Folder)
                            .label("Files"),
                    )
                    .child(
                        Button::new("files-photos")
                            .xsmall()
                            .ghost()
                            .text_color(rgb(MUTED))
                            .icon(IconName::GalleryVerticalEnd)
                            .label("Photos")
                            .disabled(!self.remote.can_photos)
                            .on_click(cx.listener(|this, _, _, cx| this.show_photos(cx))),
                    )
                    .child(
                        Button::new("remote-new-folder")
                            .xsmall()
                            .ghost()
                            .text_color(rgb(TEXT))
                            .icon(IconName::Plus)
                            .label("New folder")
                            .disabled(
                                !self.allow_upload
                                    || !self.remote.can_mkdir
                                    || self.remote.management_busy
                                    || self.remote.listed_path.as_deref()
                                        != Some(self.remote.remote_path.as_str()),
                            )
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.new_folder_side = Some(FileSide::Remote);
                                this.new_folder_input.update(cx, |input, cx| {
                                    input.set_value("", window, cx);
                                    input.focus(window, cx);
                                });
                                cx.notify();
                            })),
                    ),
            )
            .child(
                h_flex()
                    .h(px(27.))
                    .px_2()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(LINE))
                    .child(
                        Button::new("remote-back")
                            .xsmall()
                            .ghost()
                            .text_color(rgb(TEXT))
                            .icon(IconName::ChevronLeft)
                            .tooltip("Previous remote folder")
                            .disabled(back.is_none() || !available)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if let Some(path) = this.remote_back.pop() {
                                    this.navigate_remote(path, false, window, cx);
                                }
                            })),
                    )
                    .child(
                        Button::new("remote-parent")
                            .xsmall()
                            .ghost()
                            .text_color(rgb(TEXT))
                            .icon(IconName::ArrowUp)
                            .tooltip("Parent folder")
                            .disabled(parent.is_none() || !available)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if let Some(parent) = parent.clone() {
                                    this.navigate_remote(parent, true, window, cx);
                                }
                            })),
                    )
                    .child(
                        Input::new(&self.remote_path_input)
                            .xsmall()
                            .appearance(false)
                            .aria_label("Remote folder path")
                            .flex_1()
                            .min_w_0(),
                    ),
            )
            .child(self.table_header(FileSide::Remote, cx))
            .child(
                if self.remote.entries.is_empty()
                    || self.remote.listing
                    || self.remote.error.is_some()
                {
                    empty_message(message).into_any_element()
                } else {
                    uniform_list(
                        "remote-files",
                        self.remote.entries.len(),
                        cx.processor(|this, range: Range<usize>, _, cx| {
                            range
                                .map(|index| this.render_remote_row(index, cx))
                                .collect::<Vec<_>>()
                        }),
                    )
                    .flex_1()
                    .min_h_0()
                    .into_any_element()
                },
            )
            .into_any_element()
    }
}

impl FileTransferView {
    fn render_queue_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let job = &self.queue[index];
        let status = match &job.state {
            QueueState::Queued => "Queued".to_owned(),
            QueueState::Running => {
                if matches!(
                    job.action,
                    QueueAction::PhotoExport { .. } | QueueAction::PhotoUpload(_)
                ) {
                    self.remote
                        .photo_status
                        .clone()
                        .unwrap_or_else(|| "Preparing photo…".into())
                } else {
                    job.transfer_index
                        .and_then(|i| self.remote.transfers.get(i))
                        .map(|record| match record.status {
                            TransferStatus::Sent => "Verifying…".to_owned(),
                            _ => record.total.map_or_else(
                                || format_size(record.bytes),
                                |total| {
                                    format!(
                                        "{} / {}",
                                        format_size(record.bytes),
                                        format_size(total)
                                    )
                                },
                            ),
                        })
                        .unwrap_or_else(|| "Starting…".into())
                }
            }
            QueueState::Complete => {
                match job
                    .transfer_index
                    .and_then(|i| self.remote.transfers.get(i))
                {
                    Some(record) if matches!(record.status, TransferStatus::Verified) => {
                        "Verified · SHA-256".to_owned()
                    }
                    Some(record) if matches!(record.direction, TransferDirection::Upload) => {
                        "Listed · size matched".to_owned()
                    }
                    Some(_) => "Received · size matched".to_owned(),
                    None => "Complete".to_owned(),
                }
            }
            QueueState::Skipped => "Skipped".to_owned(),
            QueueState::Failed(reason) => format!("Failed: {reason}"),
            QueueState::Cancelled => "Cancelled".to_owned(),
        };
        let retry = matches!(job.state, QueueState::Failed(_) | QueueState::Cancelled);
        let retry_blocked = self.remote.download_blocked
            && matches!(
                job.action,
                QueueAction::Download { .. } | QueueAction::PhotoExport { .. }
            );
        h_flex()
            .h(px(26.))
            .px_2()
            .gap_2()
            .items_center()
            .text_xs()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_ellipsis()
                    .child(job.name.clone()),
            )
            .child(
                div()
                    .w(px(170.))
                    .text_color(rgb(MUTED))
                    .text_ellipsis()
                    .child(status),
            )
            .when(retry, |row| {
                row.child(
                    Button::new(("retry-job", index))
                        .xsmall()
                        .ghost()
                        .label("Retry")
                        .disabled(retry_blocked)
                        .tooltip(if retry_blocked {
                            "Reconnect before another download"
                        } else {
                            "Retry this item"
                        })
                        .on_click(cx.listener(move |this, _, _, cx| this.retry_job(index, cx))),
                )
            })
            .into_any_element()
    }
}

impl FileTransferView {
    fn toggle_fullscreen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.toggle_fullscreen();
        self.fullscreen_toolbar_open = false;
        cx.notify();
    }

    fn render_fullscreen_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                div()
                    .id("file-fullscreen-handle")
                    .debug_selector(|| "file-fullscreen-handle".into())
                    .occlude()
                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                        this.fullscreen_toolbar_open = *hovered;
                        cx.notify();
                    }))
                    .child(if self.fullscreen_toolbar_open {
                        h_flex()
                            .px_2()
                            .py_1()
                            .rounded_b_lg()
                            .border_1()
                            .border_color(rgb(LINE))
                            .bg(rgb(PANEL))
                            .shadow_lg()
                            .child(
                                Button::new("file-exit-fullscreen")
                                    .debug_selector(|| "file-exit-fullscreen".into())
                                    .xsmall()
                                    .ghost()
                                    .text_color(rgb(TEXT))
                                    .icon(IconName::Minimize)
                                    .label("Exit full screen")
                                    .tooltip("Exit full screen (⇧⌘F)")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.toggle_fullscreen(window, cx)
                                    })),
                            )
                            .into_any_element()
                    } else {
                        div()
                            .id("file-fullscreen-collapsed")
                            .px_6()
                            .py_1()
                            .rounded_b_md()
                            .bg(rgb(PANEL))
                            .text_color(rgb(MUTED))
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.fullscreen_toolbar_open = true;
                                cx.notify();
                            }))
                            .child(Icon::new(IconName::ChevronDown).small())
                            .into_any_element()
                    }),
            )
    }
}

impl Render for FileTransferView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let fullscreen = window.is_fullscreen();
        let caps = self.remote.caps.unwrap_or_default();
        let upload_enabled = self.allow_upload
            && caps.upload
            && self.remote.listed_path.as_deref() == Some(self.remote.remote_path.as_str())
            && self.local_selected_paths.iter().any(|path| path.is_file());
        let download_enabled = caps.download
            && !self.remote.download_blocked
            && self.remote_selected_names.iter().any(|name| {
                self.remote
                    .entries
                    .iter()
                    .any(|entry| entry.name == *name && !entry.is_dir)
            });
        let active = self
            .remote
            .transfers
            .iter()
            .any(|transfer| matches!(transfer.status, TransferStatus::Running));
        let transfer_busy = self.remote.transfers.iter().any(|transfer| {
            matches!(
                transfer.status,
                TransferStatus::Running | TransferStatus::Sent
            )
        });
        let selected_local = if self.local_selected_paths.is_empty() {
            self.local_selected.clone().into_iter().collect::<Vec<_>>()
        } else {
            self.local_selected_paths.clone()
        };
        let photo_upload_count = selected_local
            .iter()
            .filter(|path| path.is_file() && is_image_name(&path.to_string_lossy()))
            .count();
        let photo_upload_enabled = self.allow_upload
            && self.remote.can_photos
            && !self.remote.photo_busy
            && !transfer_busy
            && photo_upload_count > 0;
        let photo_download_enabled = self.remote.can_photos
            && !self.remote.photo_busy
            && !transfer_busy
            && !self.remote.download_blocked
            && !self.photo_selected_ids.is_empty()
            && self.photo_selected_ids.iter().all(|id| {
                self.remote
                    .photo_entries
                    .iter()
                    .any(|entry| entry.id == *id)
            });
        let photo_delete_enabled = self.allow_upload
            && self.remote.can_photo_delete
            && !self.remote.photo_busy
            && !self.photo_selected_ids.is_empty()
            && (self.photo_selected_ids.len() == 1 || self.remote.can_photo_batch_delete);
        v_flex()
            .size_full()
            .relative()
            .bg(rgb(BG))
            .text_color(rgb(TEXT))
            .key_context("FileTransfer")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &FileTransferFullscreen, window, cx| {
                this.toggle_fullscreen(window, cx);
            }))
            .on_action(cx.listener(|this, _: &FileTransferSelectAll, window, cx| {
                this.select_all(window, cx);
            }))
            .when(!fullscreen, |this| {
                this.child(
                    TitleBar::new().child(
                        div()
                            .w_full()
                            .text_center()
                            .text_sm()
                            .font_semibold()
                            .child(format!("File transfer — {}", self.host)),
                    ),
                )
            })
            .child(
                h_flex()
                    .h(px(32.))
                    .px_2()
                    .gap_1()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(LINE))
                    .bg(rgb(PANEL))
                    .child(if self.photo_mode {
                        Button::new("upload-to-photos")
                            .xsmall()
                            .ghost()
                            .text_color(rgb(TEXT))
                            .icon(IconName::ArrowUp)
                            .label(if photo_upload_count > 1 {
                                format!("Upload to Photos ({photo_upload_count})")
                            } else {
                                "Upload to Photos".into()
                            })
                            .disabled(!photo_upload_enabled)
                            .on_click(cx.listener(|this, _, _, cx| this.upload_to_photos(cx)))
                            .into_any_element()
                    } else {
                        Button::new("upload")
                            .xsmall()
                            .ghost()
                            .text_color(rgb(TEXT))
                            .icon(IconName::ArrowUp)
                            .label("Upload")
                            .disabled(!upload_enabled)
                            .tooltip("Upload selected local file")
                            .on_click(cx.listener(|this, _, _, cx| this.upload(cx)))
                            .into_any_element()
                    })
                    .child(if self.photo_mode {
                        Button::new("download-photo")
                            .xsmall()
                            .ghost()
                            .text_color(rgb(TEXT))
                            .icon(IconName::ArrowDown)
                            .label(if self.photo_selected_ids.len() > 1 {
                                format!("Download ({})", self.photo_selected_ids.len())
                            } else {
                                "Download".into()
                            })
                            .disabled(!photo_download_enabled)
                            .on_click(cx.listener(|this, _, _, cx| this.download_photo(cx)))
                            .into_any_element()
                    } else {
                        Button::new("download")
                            .xsmall()
                            .ghost()
                            .text_color(rgb(TEXT))
                            .icon(IconName::ArrowDown)
                            .label("Download")
                            .disabled(!download_enabled)
                            .tooltip("Download selected remote file into the local folder")
                            .on_click(cx.listener(|this, _, _, cx| this.download(cx)))
                            .into_any_element()
                    })
                    .when(self.photo_mode && self.remote.can_photo_delete, |this| {
                        this.child(
                            Button::new("delete-selected-photos")
                                .xsmall()
                                .ghost()
                                .text_color(rgb(TEXT))
                                .icon(IconName::Delete)
                                .label(if self.photo_selected_ids.len() > 1 {
                                    format!("Delete ({})", self.photo_selected_ids.len())
                                } else {
                                    "Delete".into()
                                })
                                .disabled(!photo_delete_enabled)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(client) = &this.client {
                                        client.send(FileCommand::PhotoDelete(
                                            this.photo_selected_ids.clone(),
                                        ));
                                    }
                                    cx.notify();
                                })),
                        )
                    })
                    .when(
                        !self.photo_mode && !self.local_selected_paths.is_empty(),
                        |this| {
                            this.child(
                                Button::new("delete-local-selected")
                                    .xsmall()
                                    .ghost()
                                    .text_color(rgb(TEXT))
                                    .icon(IconName::Delete)
                                    .label(format!(
                                        "Delete local ({})",
                                        self.local_selected_paths.len()
                                    ))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.pending_batch_delete = Some(FileSide::Local);
                                        cx.notify();
                                    })),
                            )
                        },
                    )
                    .when(
                        !self.photo_mode
                            && self.allow_upload
                            && self.remote.can_delete
                            && !self.remote_selected_names.is_empty(),
                        |this| {
                            this.child(
                                Button::new("delete-remote-selected")
                                    .xsmall()
                                    .ghost()
                                    .text_color(rgb(TEXT))
                                    .icon(IconName::Delete)
                                    .label(format!(
                                        "Delete remote ({})",
                                        self.remote_selected_names.len()
                                    ))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.pending_batch_delete = Some(FileSide::Remote);
                                        cx.notify();
                                    })),
                            )
                        },
                    )
                    .child(
                        Button::new("refresh")
                            .xsmall()
                            .ghost()
                            .text_color(rgb(TEXT))
                            .icon(IconName::Replace)
                            .label("Refresh")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.reload_local(cx);
                                if this.photo_mode {
                                    if let Some(client) = &this.client {
                                        client.send(FileCommand::PhotoList {
                                            offset: this.remote.photo_offset,
                                            album: this.remote.photo_album.clone(),
                                        });
                                    }
                                } else {
                                    this.refresh_remote();
                                }
                            })),
                    )
                    .when(active, |this| {
                        this.child(
                            Button::new("cancel-transfer")
                                .xsmall()
                                .ghost()
                                .text_color(rgb(TEXT))
                                .icon(IconName::Close)
                                .label("Cancel")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(client) = &this.client {
                                        client.send(FileCommand::Cancel);
                                    }
                                    cx.notify();
                                })),
                        )
                    })
                    .child(div().flex_1())
                    .child(
                        Button::new("transfers-toggle")
                            .xsmall()
                            .ghost()
                            .text_color(rgb(TEXT))
                            .icon(if self.transfers_open {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .label("Transfers")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.transfers_open = !this.transfers_open;
                                this.auto_collapse_armed = false;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_stretch()
                    .child(self.render_local_pane(cx))
                    .child(div().w(px(1.)).h_full().bg(rgb(LINE)))
                    .child(if self.photo_mode {
                        self.render_photos_pane(cx)
                    } else {
                        self.render_remote_pane(cx)
                    }),
            )
            .when(self.transfers_open, |this| {
                this.child(
                    v_flex()
                        .h(px(112.))
                        .border_t_1()
                        .border_color(rgb(LINE))
                        .bg(rgb(PANEL))
                        .child(
                            h_flex()
                                .h(px(27.))
                                .px_2()
                                .items_center()
                                .font_semibold()
                                .text_xs()
                                .child(format!(
                                    "Transfers ({})",
                                    if self.queue.is_empty() {
                                        self.remote.transfers.len()
                                    } else {
                                        self.queue.len()
                                    }
                                )),
                        )
                        .child(if !self.queue.is_empty() {
                            uniform_list(
                                "transfer-queue",
                                self.queue.len(),
                                cx.processor(|this, range: Range<usize>, _, cx| {
                                    range
                                        .map(|row| {
                                            this.render_queue_row(this.queue.len() - 1 - row, cx)
                                        })
                                        .collect::<Vec<_>>()
                                }),
                            )
                            .flex_1()
                            .min_h_0()
                            .into_any_element()
                        } else if self.remote.transfers.is_empty() {
                            empty_message("No transfers yet").into_any_element()
                        } else {
                            v_flex()
                                .flex_1()
                                .overflow_hidden()
                                .children(self.remote.transfers.iter().rev().take(3).map(
                                    |transfer| {
                                        let status = match &transfer.status {
                                            TransferStatus::Running => transfer.total.map_or_else(
                                                || format_size(transfer.bytes),
                                                |total| {
                                                    format!(
                                                        "{} / {}",
                                                        format_size(transfer.bytes),
                                                        format_size(total)
                                                    )
                                                },
                                            ),
                                            TransferStatus::Complete => {
                                                if matches!(
                                                    transfer.direction,
                                                    TransferDirection::Upload
                                                ) {
                                                    "Complete · listed".into()
                                                } else {
                                                    "Complete".into()
                                                }
                                            }
                                            TransferStatus::Verified => "Verified · SHA-256".into(),
                                            TransferStatus::Sent => {
                                                if matches!(
                                                    transfer.direction,
                                                    TransferDirection::Upload
                                                ) {
                                                    "Sent · verifying".into()
                                                } else {
                                                    "Received · verifying".into()
                                                }
                                            }
                                            TransferStatus::Cancelled => "Cancelled".into(),
                                            TransferStatus::Failed(reason) => {
                                                format!("Failed: {reason}")
                                            }
                                        };
                                        h_flex()
                                            .h(px(26.))
                                            .px_2()
                                            .gap_2()
                                            .items_center()
                                            .text_xs()
                                            .child(
                                                Icon::new(
                                                    if matches!(
                                                        transfer.direction,
                                                        TransferDirection::Upload
                                                    ) {
                                                        IconName::ArrowUp
                                                    } else {
                                                        IconName::ArrowDown
                                                    },
                                                )
                                                .small(),
                                            )
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .text_ellipsis()
                                                    .child(transfer.name.clone()),
                                            )
                                            .child(
                                                div()
                                                    .w(px(170.))
                                                    .text_color(rgb(MUTED))
                                                    .text_ellipsis()
                                                    .child(status),
                                            )
                                    },
                                ))
                                .into_any_element()
                        }),
                )
            })
            .child(
                h_flex()
                    .h(px(22.))
                    .px_2()
                    .items_center()
                    .border_t_1()
                    .border_color(rgb(LINE))
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .child(
                        div().w_full().text_ellipsis().child(
                            self.status_error
                                .clone()
                                .or_else(|| {
                                    self.photo_mode
                                        .then(|| {
                                            self.remote
                                                .photo_error
                                                .clone()
                                                .or_else(|| self.remote.photo_status.clone())
                                        })
                                        .flatten()
                                })
                                .or_else(|| self.remote.error.clone())
                                .unwrap_or_else(|| {
                                    format!(
                                        "{} local items · {} remote items",
                                        self.local_entries.len(),
                                        self.remote.entries.len()
                                    )
                                }),
                        ),
                    ),
            )
            .when(fullscreen, |this| {
                this.child(self.render_fullscreen_toolbar(cx))
            })
            .when_some(self.details.clone(), |this, details| {
                this.child(render_file_details(details, cx))
            })
            .when_some(self.pending_delete.clone(), |this, details| {
                this.child(render_delete_confirmation(details, cx))
            })
            .when_some(self.new_folder_side, |this, side| {
                this.child(render_new_folder(side, &self.new_folder_input, cx))
            })
            .when_some(self.pending_rename.clone(), |this, details| {
                this.child(render_rename(details, &self.new_folder_input, cx))
            })
            .when_some(self.pending_conflicts.as_ref(), |this, jobs| {
                let requires_remote_replace = jobs.iter().any(|job| match &job.action {
                    QueueAction::Upload { remote, .. } => {
                        self.remote
                            .entries
                            .iter()
                            .any(|entry| remote.ends_with(&format!("/{}", entry.name)))
                            || remote
                                .rsplit('/')
                                .next()
                                .is_some_and(|name| self.reserved_remote_names().contains(name))
                    }
                    _ => false,
                });
                this.child(render_conflict_confirmation(
                    jobs.len(),
                    !requires_remote_replace || self.remote.can_replace,
                    cx,
                ))
            })
            .when_some(self.pending_batch_delete, |this, side| {
                let count = if side == FileSide::Local {
                    self.local_selected_paths.len()
                } else {
                    self.remote_selected_names.len()
                };
                this.child(render_batch_delete_confirmation(count, side, cx))
            })
    }
}

fn album_menu(
    view: Entity<FileTransferView>,
    albums: Vec<PhotoAlbum>,
) -> impl Fn(
    gpui_component::menu::PopupMenu,
    &mut Window,
    &mut Context<gpui_component::menu::PopupMenu>,
) -> gpui_component::menu::PopupMenu {
    move |menu, _, _| {
        let all_view = view.clone();
        let mut menu = menu.item(PopupMenuItem::new("All photos").on_click(move |_, _, cx| {
            all_view.update(cx, |this, cx| {
                this.photo_selected = None;
                this.photo_selected_ids.clear();
                this.photo_anchor = None;
                if let Some(client) = &this.client {
                    client.send(FileCommand::PhotoList {
                        offset: 0,
                        album: None,
                    });
                }
                cx.notify();
            });
        }));
        for album in &albums {
            let album_view = view.clone();
            let id = album.id.clone();
            menu = menu.item(
                PopupMenuItem::new(album.name.clone()).on_click(move |_, _, cx| {
                    album_view.update(cx, |this, cx| {
                        this.photo_selected = None;
                        this.photo_selected_ids.clear();
                        this.photo_anchor = None;
                        if let Some(client) = &this.client {
                            client.send(FileCommand::PhotoList {
                                offset: 0,
                                album: Some(id.clone()),
                            });
                        }
                        cx.notify();
                    });
                }),
            );
        }
        menu
    }
}

fn photo_menu(
    view: Entity<FileTransferView>,
    photo: PhotoEntry,
    can_delete: bool,
    can_batch: bool,
    selected_ids: Vec<String>,
) -> impl Fn(
    gpui_component::menu::PopupMenu,
    &mut Window,
    &mut Context<gpui_component::menu::PopupMenu>,
) -> gpui_component::menu::PopupMenu {
    move |menu, _, _| {
        let photo_view = view.clone();
        let selected = photo.id.clone();
        let menu = menu.item(PopupMenuItem::new("Download").on_click(move |_, _, cx| {
            photo_view.update(cx, |this, cx| {
                if !this.photo_selected_ids.contains(&selected) {
                    this.photo_selected_ids = vec![selected.clone()];
                }
                this.photo_selected = Some(selected.clone());
                this.download_photo(cx);
            });
        }));
        if can_delete {
            let delete_view = view.clone();
            let id = photo.id.clone();
            let batch = can_batch && selected_ids.len() > 1 && selected_ids.contains(&id);
            let targets = if batch {
                selected_ids.clone()
            } else {
                vec![id.clone()]
            };
            let label = if batch {
                format!("Delete selected ({})…", targets.len())
            } else {
                "Delete…".into()
            };
            menu.item(PopupMenuItem::new(label).on_click(move |_, _, cx| {
                delete_view.update(cx, |this, cx| {
                    if this.allow_upload
                        && this.remote.can_photo_delete
                        && !this.remote.photo_busy
                        && let Some(client) = &this.client
                    {
                        client.send(FileCommand::PhotoDelete(targets.clone()));
                    }
                    cx.notify();
                });
            }))
        } else {
            menu
        }
    }
}

fn file_menu(
    view: Entity<FileTransferView>,
    details: FileDetails,
    can_transfer: bool,
    can_delete: bool,
    can_rename: bool,
    can_photo: bool,
) -> impl Fn(
    gpui_component::menu::PopupMenu,
    &mut Window,
    &mut Context<gpui_component::menu::PopupMenu>,
) -> gpui_component::menu::PopupMenu {
    move |menu, _, _| {
        let name = details.name.clone();
        let path = details.path.clone();
        let info = details.clone();
        let action = details.clone();
        let info_view = view.clone();
        let action_view = view.clone();
        let menu = menu
            .item(PopupMenuItem::new("Copy Name").on_click(move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(name.clone()));
            }))
            .item(PopupMenuItem::new("Copy Path").on_click(move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(path.clone()));
            }))
            .item(PopupMenuItem::new("Get Info…").on_click(move |_, _, cx| {
                info_view.update(cx, |this, cx| {
                    this.details = Some(info.clone());
                    cx.notify();
                });
            }));
        let menu = if action.is_dir {
            menu.separator()
                .item(
                    PopupMenuItem::new("Open Folder").on_click(move |_, window, cx| {
                        action_view.update(cx, |this, cx| match action.side {
                            FileSide::Local => {
                                this.navigate_local(PathBuf::from(&action.path), true, window, cx)
                            }
                            FileSide::Remote => {
                                this.navigate_remote(action.path.clone(), true, window, cx)
                            }
                        });
                    }),
                )
        } else if can_transfer {
            let label = match action.side {
                FileSide::Local => "Upload",
                FileSide::Remote => "Download",
            };
            menu.separator()
                .item(PopupMenuItem::new(label).on_click(move |_, _, cx| {
                    action_view.update(cx, |this, cx| match action.side {
                        FileSide::Local => {
                            this.local_selected = Some(PathBuf::from(&action.path));
                            this.local_selected_paths = vec![PathBuf::from(&action.path)];
                            this.upload(cx);
                        }
                        FileSide::Remote => {
                            this.remote_selected = Some(action.name.clone());
                            this.remote_selected_names = vec![action.name.clone()];
                            this.download(cx);
                        }
                    });
                }))
        } else {
            menu
        };
        let menu = if can_rename {
            let rename_view = view.clone();
            let rename_details = details.clone();
            menu.separator().item(
                PopupMenuItem::new("Rename…").on_click(move |_, window, cx| {
                    rename_view.update(cx, |this, cx| {
                        this.pending_rename = Some(rename_details.clone());
                        this.new_folder_input.update(cx, |input, cx| {
                            input.set_value(rename_details.name.clone(), window, cx);
                            input.focus(window, cx);
                        });
                        cx.notify();
                    });
                }),
            )
        } else {
            menu
        };
        let menu = if can_photo {
            let photo_view = view.clone();
            let photo_details = details.clone();
            let label = if details.side == FileSide::Local {
                "Upload to Photos"
            } else {
                "Import to Photos"
            };
            menu.separator()
                .item(PopupMenuItem::new(label).on_click(move |_, _, cx| {
                    photo_view.update(cx, |this, cx| match photo_details.side {
                        FileSide::Local => {
                            this.local_selected = Some(PathBuf::from(&photo_details.path));
                            this.upload_to_photos(cx);
                        }
                        FileSide::Remote => {
                            this.import_remote_photo(photo_details.path.clone(), cx)
                        }
                    });
                }))
        } else {
            menu
        };
        if can_delete {
            let delete_view = view.clone();
            let delete_details = details.clone();
            menu.separator()
                .item(PopupMenuItem::new("Delete…").on_click(move |_, _, cx| {
                    delete_view.update(cx, |this, cx| {
                        this.pending_delete = Some(delete_details.clone());
                        cx.notify();
                    });
                }))
        } else {
            menu
        }
    }
}

fn render_delete_confirmation(
    details: FileDetails,
    cx: &mut Context<FileTransferView>,
) -> AnyElement {
    let location = if details.side == FileSide::Local {
        "this Mac"
    } else {
        "TrollVNC"
    };
    div()
        .id("delete-confirmation-overlay")
        .occlude()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgba(0x000000aa))
        .child(
            v_flex()
                .w(px(420.))
                .rounded_lg()
                .border_1()
                .border_color(rgb(LINE))
                .bg(rgb(PANEL))
                .p_4()
                .gap_3()
                .child(
                    div()
                        .font_semibold()
                        .child(format!("Delete {}?", details.name)),
                )
                .child(div().text_xs().text_color(rgb(MUTED)).child(format!(
                    "Permanently remove this item from {location}. Folders must be empty."
                )))
                .child(
                    h_flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("delete-cancel")
                                .xsmall()
                                .label("Cancel")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.pending_delete = None;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("delete-confirm")
                                .xsmall()
                                .label("Delete permanently")
                                .on_click(cx.listener(|this, _, _, cx| this.confirm_delete(cx))),
                        ),
                ),
        )
        .into_any_element()
}

fn render_conflict_confirmation(
    count: usize,
    can_replace: bool,
    cx: &mut Context<FileTransferView>,
) -> AnyElement {
    div().id("file-conflict-overlay").occlude().absolute().inset_0()
        .flex().items_center().justify_center().bg(rgba(0x000000aa))
        .child(v_flex().w(px(440.)).rounded_lg().border_1().border_color(rgb(LINE))
            .bg(rgb(PANEL)).p_4().gap_3()
            .child(div().font_semibold().child("Files with the same name"))
            .child(div().text_xs().text_color(rgb(MUTED)).child(format!("Some of the {count} selected files already exist at the destination. Choose how to handle those files.")))
            .child(h_flex().justify_end().gap_2()
                .child(Button::new("conflict-cancel").xsmall().label("Cancel")
                    .on_click(cx.listener(|this, _, _, cx| { this.pending_conflicts = None; cx.notify(); })))
                .child(Button::new("conflict-skip").xsmall().label("Skip")
                    .on_click(cx.listener(|this, _, _, cx| this.resolve_conflicts(CollisionPolicy::Skip, cx))))
                .child(Button::new("conflict-rename").xsmall().label("Rename")
                    .on_click(cx.listener(|this, _, _, cx| this.resolve_conflicts(CollisionPolicy::Rename, cx))))
                .child(Button::new("conflict-replace").xsmall().label("Replace")
                    .disabled(!can_replace)
                    .on_click(cx.listener(|this, _, _, cx| this.resolve_conflicts(CollisionPolicy::Replace, cx)))))
        ).into_any_element()
}

fn render_batch_delete_confirmation(
    count: usize,
    side: FileSide,
    cx: &mut Context<FileTransferView>,
) -> AnyElement {
    let location = if side == FileSide::Local {
        "this Mac"
    } else {
        "TrollVNC"
    };
    div()
        .id("batch-delete-overlay")
        .occlude()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgba(0x000000aa))
        .child(
            v_flex()
                .w(px(420.))
                .rounded_lg()
                .border_1()
                .border_color(rgb(LINE))
                .bg(rgb(PANEL))
                .p_4()
                .gap_3()
                .child(
                    div()
                        .font_semibold()
                        .child(format!("Delete {count} items from {location}?")),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(MUTED))
                        .child("Items will be deleted one at a time. Folders must be empty."),
                )
                .child(
                    h_flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("batch-delete-cancel")
                                .xsmall()
                                .label("Cancel")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.pending_batch_delete = None;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("batch-delete-confirm")
                                .xsmall()
                                .label("Delete permanently")
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.confirm_batch_delete(cx)),
                                ),
                        ),
                ),
        )
        .into_any_element()
}

fn render_new_folder(
    side: FileSide,
    input: &Entity<InputState>,
    cx: &mut Context<FileTransferView>,
) -> AnyElement {
    let location = if side == FileSide::Local {
        "This Mac"
    } else {
        "TrollVNC"
    };
    div()
        .id("new-folder-overlay")
        .occlude()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgba(0x000000aa))
        .child(
            v_flex()
                .w(px(420.))
                .rounded_lg()
                .border_1()
                .border_color(rgb(LINE))
                .bg(rgb(PANEL))
                .p_4()
                .gap_3()
                .child(
                    div()
                        .font_semibold()
                        .child(format!("New folder · {location}")),
                )
                .child(Input::new(input).aria_label("New folder name").w_full())
                .child(
                    h_flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("new-folder-cancel")
                                .xsmall()
                                .label("Cancel")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.new_folder_side = None;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("new-folder-create")
                                .xsmall()
                                .label("Create")
                                .on_click(cx.listener(|this, _, _, cx| this.create_folder(cx))),
                        ),
                ),
        )
        .into_any_element()
}

fn render_rename(
    details: FileDetails,
    input: &Entity<InputState>,
    cx: &mut Context<FileTransferView>,
) -> AnyElement {
    div()
        .id("rename-overlay")
        .occlude()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgba(0x000000aa))
        .child(
            v_flex()
                .w(px(420.))
                .rounded_lg()
                .border_1()
                .border_color(rgb(LINE))
                .bg(rgb(PANEL))
                .p_4()
                .gap_3()
                .child(
                    div()
                        .font_semibold()
                        .child(format!("Rename {}", details.name)),
                )
                .child(Input::new(input).aria_label("New name").w_full())
                .child(
                    h_flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("rename-cancel")
                                .xsmall()
                                .label("Cancel")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.pending_rename = None;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("rename-confirm")
                                .xsmall()
                                .label("Rename")
                                .on_click(cx.listener(|this, _, _, cx| this.rename_item(cx))),
                        ),
                ),
        )
        .into_any_element()
}

fn render_file_details(details: FileDetails, cx: &mut Context<FileTransferView>) -> AnyElement {
    let side = match details.side {
        FileSide::Local => "This Mac",
        FileSide::Remote => "TrollVNC",
    };
    div()
        .id("file-info-overlay")
        .occlude()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgba(0x000000aa))
        .on_click(cx.listener(|this, _, _, cx| {
            this.details = None;
            cx.notify();
        }))
        .child(
            v_flex()
                .id("file-info-card")
                .w(px(450.))
                .max_w(relative(0.9))
                .rounded_lg()
                .border_1()
                .border_color(rgb(LINE))
                .bg(rgb(PANEL))
                .shadow_lg()
                .p_4()
                .gap_3()
                .on_click(|_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .font_semibold()
                        .text_sm()
                        .child(format!("{} · Info", details.name)),
                )
                .child(info_row("Location", side))
                .child(info_row(
                    "Type",
                    if details.is_dir { "Folder" } else { "File" },
                ))
                .child(info_row("Path", details.path))
                .child(info_row(
                    "Size",
                    details.size.map_or_else(
                        || "—".into(),
                        |size| format!("{} ({} bytes)", format_size(size), size),
                    ),
                ))
                .child(info_row(
                    "Modified",
                    details.modified.map_or_else(
                        || "—".into(),
                        |time| {
                            DateTime::<Local>::from(time)
                                .format("%Y-%m-%d %H:%M:%S")
                                .to_string()
                        },
                    ),
                ))
                .child(
                    h_flex().justify_end().child(
                        Button::new("file-info-close")
                            .xsmall()
                            .label("Close")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.details = None;
                                cx.notify();
                            })),
                    ),
                ),
        )
        .into_any_element()
}

fn info_row(label: &'static str, value: impl Into<SharedString>) -> impl IntoElement {
    h_flex()
        .gap_2()
        .items_start()
        .text_xs()
        .child(div().w(px(76.)).text_color(rgb(MUTED)).child(label))
        .child(div().flex_1().min_w_0().child(value.into()))
}

impl FileTransferView {
    fn table_header(&self, side: FileSide, cx: &mut Context<Self>) -> impl IntoElement {
        let sort = if side == FileSide::Local {
            self.local_sort
        } else {
            self.remote_sort
        };
        let label = |column, name: &str| {
            if sort.column == column {
                format!("{} {}", name, if sort.ascending { "↑" } else { "↓" })
            } else {
                name.to_owned()
            }
        };
        h_flex()
            .h(px(23.))
            .px_2()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(rgb(LINE))
            .text_xs()
            .text_color(rgb(MUTED))
            .child(div().w(px(14.)))
            .child(
                div()
                    .id(("sort-name", side as usize))
                    .flex_1()
                    .cursor_pointer()
                    .child(label(SortColumn::Name, "Name"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.change_sort(side, SortColumn::Name, cx)
                    })),
            )
            .child(
                div()
                    .id(("sort-size", side as usize))
                    .w(px(70.))
                    .cursor_pointer()
                    .child(label(SortColumn::Size, "Size"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.change_sort(side, SortColumn::Size, cx)
                    })),
            )
            .child(
                div()
                    .id(("sort-modified", side as usize))
                    .w(px(92.))
                    .cursor_pointer()
                    .child(label(SortColumn::Modified, "Modified"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.change_sort(side, SortColumn::Modified, cx)
                    })),
            )
    }
}

fn join_remote(dir: &str, name: &str) -> String {
    if dir == "/" {
        format!("/{name}")
    } else {
        format!("{dir}/{name}")
    }
}

fn unique_copy_name(name: &str, occupied: &HashSet<String>) -> String {
    let path = Path::new(name);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(name);
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| format!(".{s}"))
        .unwrap_or_default();
    for number in 1..=10_000 {
        let candidate = format!("{stem} ({number}){extension}");
        if !occupied.contains(&candidate) {
            return candidate;
        }
    }
    format!("{stem}-{}{extension}", std::process::id())
}

fn photo_staging_path(local_dir: &Path, occupied: &mut HashSet<String>) -> PathBuf {
    let base = ".rv-photo-download";
    let mut candidate = unique_copy_name(base, occupied);
    while local_dir.join(&candidate).exists() {
        occupied.insert(candidate);
        candidate = unique_copy_name(base, occupied);
    }
    occupied.insert(candidate.clone());
    local_dir.join(candidate)
}

#[cfg(target_os = "macos")]
fn rename_local_no_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;

    unsafe extern "C" {
        fn renameatx_np(
            from_fd: std::ffi::c_int,
            from: *const std::ffi::c_char,
            to_fd: std::ffi::c_int,
            to: *const std::ffi::c_char,
            flags: std::ffi::c_uint,
        ) -> std::ffi::c_int;
    }

    let parent = from
        .parent()
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    if to.parent() != Some(parent) {
        return Err(std::io::ErrorKind::InvalidInput.into());
    }
    let dir = std::fs::File::open(parent)?;
    let old = CString::new(
        from.file_name()
            .ok_or(std::io::ErrorKind::InvalidInput)?
            .as_bytes(),
    )
    .map_err(|_| std::io::ErrorKind::InvalidInput)?;
    let new = CString::new(
        to.file_name()
            .ok_or(std::io::ErrorKind::InvalidInput)?
            .as_bytes(),
    )
    .map_err(|_| std::io::ErrorKind::InvalidInput)?;
    // Darwin's RENAME_EXCL refuses to overwrite an existing name atomically.
    let result = unsafe {
        renameatx_np(
            dir.as_raw_fd(),
            old.as_ptr(),
            dir.as_raw_fd(),
            new.as_ptr(),
            4,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(target_os = "macos"))]
fn rename_local_no_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    if std::fs::symlink_metadata(to).is_ok() {
        return Err(std::io::ErrorKind::AlreadyExists.into());
    }
    std::fs::rename(from, to)
}

fn remote_parent(path: &str) -> Option<String> {
    if path == "/" {
        return None;
    }
    let parent = path.rsplit_once('/')?.0;
    Some(if parent.is_empty() {
        "/".into()
    } else {
        parent.into()
    })
}

fn empty_message(message: impl Into<SharedString>) -> impl IntoElement {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .text_xs()
        .text_color(rgb(MUTED))
        .child(message.into())
}

fn read_local_dir(path: &Path) -> Result<Vec<LocalEntry>, String> {
    let entries = std::fs::read_dir(path).map_err(|error| error.to_string())?;
    let mut files = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            let is_dir = metadata.is_dir();
            Some(LocalEntry {
                path: entry.path(),
                name: entry.file_name().to_string_lossy().into_owned(),
                is_dir,
                size_bytes: (!is_dir).then_some(metadata.len()),
                modified_time: metadata.modified().ok(),
                size: if is_dir {
                    "—".into()
                } else {
                    format_size(metadata.len())
                },
                modified: metadata
                    .modified()
                    .map(relative_time)
                    .unwrap_or_else(|_| "—".into()),
            })
        })
        .collect::<Vec<_>>();
    files.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(files)
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = "B";
    for next in ["KB", "MB", "GB", "TB"] {
        value /= 1024.0;
        unit = next;
        if value < 1024.0 {
            break;
        }
    }
    format!("{value:.1} {unit}")
}

fn relative_time(time: SystemTime) -> String {
    match SystemTime::now().duration_since(time) {
        Ok(age) if age < Duration::from_secs(86_400) => "Today".into(),
        Ok(age) if age < Duration::from_secs(172_800) => "Yesterday".into(),
        Ok(age) if age < Duration::from_secs(7 * 86_400) => {
            format!("{}d ago", age.as_secs() / 86_400)
        }
        Ok(_) => DateTime::<Local>::from(time).format("%Y-%m-%d").to_string(),
        Err(_) => "—".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        FileSide, FileSort, FileTransferView, PHOTO_PAGE_SIZE, QueueAction, QueueJob, QueueState,
        TransferMemory, read_local_dir, relative_time, rename_local_no_replace,
    };
    use rv_core::TransferFolders;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;
    use std::time::{Duration, SystemTime};

    fn test_memory() -> TransferMemory {
        TransferMemory {
            folders: Rc::new(RefCell::new(TransferFolders::default())),
            address_book: None,
            connection_id: None,
        }
    }

    #[test]
    fn old_file_time_uses_calendar_date() {
        assert_eq!(
            relative_time(SystemTime::now() - Duration::from_secs(6 * 86_400)),
            "6d ago"
        );
        let old = relative_time(SystemTime::now() - Duration::from_secs(8 * 86_400));
        assert_eq!(old.len(), 10);
        assert_eq!(old.as_bytes()[4], b'-');
        assert_eq!(old.as_bytes()[7], b'-');
    }

    #[test]
    fn local_rename_never_overwrites_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        let target = dir.path().join("target.txt");
        std::fs::write(&source, b"source").unwrap();
        std::fs::write(&target, b"target").unwrap();
        assert!(rename_local_no_replace(&source, &target).is_err());
        assert_eq!(std::fs::read(&source).unwrap(), b"source");
        assert_eq!(std::fs::read(&target).unwrap(), b"target");
    }
    use gpui::{AppContext, Modifiers, TestAppContext, px, size};
    use gpui_component::input::InputState;
    use rv_session::{FileTransferSnapshot, PhotoEntry, SessionHandle};
    use vnc::tight::{TightFileCaps, TightFileEntry};

    #[gpui::test]
    fn reopening_transfer_view_restores_folders_and_photos_mode(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let nested = directory.path().join("last-visited");
        std::fs::create_dir(&nested).unwrap();
        let memory = test_memory();
        let (session, _peer) = SessionHandle::test_pair();

        cx.update(gpui_component::init);
        let mut first = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| {
                FileTransferView::new(
                    "example.test:5900".into(),
                    session.file_client(),
                    true,
                    memory.clone(),
                    window,
                    cx,
                )
            });
            first = Some(view.clone());
            gpui_component::Root::new(view, window, cx)
        });
        let first = first.unwrap();
        cx.update(|window, cx| {
            first.update(cx, |view, cx| {
                view.navigate_local(nested.clone(), true, window, cx);
                view.navigate_remote("/last-visited".into(), true, window, cx);
                view.remote.can_photos = true;
                view.show_photos(cx);
            });
        });
        assert_eq!(memory.folders.borrow().local.as_ref(), Some(&nested));
        assert_eq!(
            memory.folders.borrow().remote.as_deref(),
            Some("/last-visited")
        );
        assert!(memory.folders.borrow().photo_mode);

        let reopened = cx.update(|window, cx| {
            cx.new(|cx| {
                FileTransferView::new(
                    "example.test:5900".into(),
                    session.file_client(),
                    true,
                    memory.clone(),
                    window,
                    cx,
                )
            })
        });
        reopened.read_with(cx, |view, cx| {
            assert_eq!(view.local_dir, nested);
            assert_eq!(
                view.local_path_input.read(cx).value().as_ref(),
                nested.to_str().unwrap()
            );
            assert_eq!(
                view.remote_path_input.read(cx).value().as_ref(),
                "/last-visited"
            );
            // The capability may arrive after this view is constructed.
            assert!(!view.photo_mode);
        });
        reopened.update(cx, |view, cx| {
            view.remote.can_photos = true;
            view.sync_photo_mode(cx);
            assert!(view.photo_mode);
            view.show_files(cx);
        });
        assert!(!memory.folders.borrow().photo_mode);
    }

    #[gpui::test]
    fn compact_window_renders_a_large_directory_and_fullscreen_exit(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        for index in 0..500 {
            std::fs::write(
                directory.path().join(format!("file-{index:04}.txt")),
                b"test",
            )
            .unwrap();
        }
        let entries = read_local_dir(directory.path()).unwrap();
        assert_eq!(entries.len(), 500);
        let remote = FileTransferSnapshot {
            caps: Some(TightFileCaps {
                list: true,
                upload: true,
                download: true,
            }),
            listed_path: Some("/".into()),
            entries: (0..500)
                .map(|index| TightFileEntry {
                    name: format!("remote-{index:04}.txt"),
                    is_dir: false,
                    size: 4,
                    modified: 0,
                })
                .collect(),
            ..Default::default()
        };

        cx.update(|cx| {
            gpui_component::init(cx);
            crate::bind_keys(cx);
        });
        let mut view_entity = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let focus = cx.focus_handle();
            focus.focus(window, cx);
            let local_path_input = cx.new(|cx| InputState::new(window, cx));
            let remote_path_input = cx.new(|cx| InputState::new(window, cx));
            let new_folder_input = cx.new(|cx| InputState::new(window, cx));
            let view = cx.new(|_| FileTransferView {
                host: "example.test:5901".into(),
                local_dir: directory.path().to_path_buf(),
                local_path_input,
                remote_path_input,
                local_back: Vec::new(),
                remote_back: Vec::new(),
                local_entries: entries,
                local_selected: None,
                local_selected_paths: Vec::new(),
                local_anchor: None,
                local_sort: FileSort::default(),
                local_loading: false,
                local_error: None,
                transfers_open: true,
                queue: (0..40)
                    .map(|index| {
                        QueueJob::new(
                            format!("queued-{index}.txt"),
                            QueueAction::DeleteLocal(
                                directory.path().join(format!("queued-{index}.txt")),
                            ),
                        )
                    })
                    .collect(),
                current_job: None,
                pending_conflicts: None,
                pending_batch_delete: None,
                queue_idle_since: None,
                auto_collapse_armed: false,
                client: None,
                remote,
                preferred_remote: None,
                memory: test_memory(),
                remote_selected: None,
                remote_selected_names: Vec::new(),
                remote_anchor: None,
                remote_sort: FileSort::default(),
                active_side: FileSide::Remote,
                photo_mode: false,
                photo_selected: None,
                photo_selected_ids: Vec::new(),
                photo_anchor: None,
                photo_thumbs: HashMap::new(),
                allow_upload: true,
                details: None,
                pending_delete: None,
                pending_rename: None,
                new_folder_side: None,
                new_folder_input,
                status_error: None,
                fullscreen_toolbar_open: false,
                focus,
                _subscriptions: Vec::new(),
            });
            view_entity = Some(view.clone());
            gpui_component::Root::new(view, window, cx)
        });
        cx.simulate_resize(size(px(760.), px(440.)));
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        cx.update(|window, cx| {
            window.toggle_fullscreen();
            assert!(window.is_fullscreen());
            window.draw(cx).clear(cx);
        });
        let handle = cx.debug_bounds("file-fullscreen-handle").unwrap();
        cx.simulate_click(handle.center(), Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let exit = cx.debug_bounds("file-exit-fullscreen").unwrap();
        cx.simulate_click(exit.center(), Modifiers::default());
        cx.update(|window, _| assert!(!window.is_fullscreen()));

        cx.simulate_keystrokes("cmd-shift-f");
        cx.update(|window, _| assert!(window.is_fullscreen()));
        cx.simulate_keystrokes("cmd-shift-f");
        cx.update(|window, _| assert!(!window.is_fullscreen()));

        let command = Modifiers {
            platform: true,
            ..Default::default()
        };
        for (side, expected) in [
            ("local-file", "file-0001.txt"),
            ("remote-file", "remote-0001.txt"),
        ] {
            cx.update(|window, cx| window.draw(cx).clear(cx));
            let first = cx
                .debug_bounds(if side == "local-file" {
                    "local-file-0"
                } else {
                    "remote-file-0"
                })
                .unwrap();
            let second = cx
                .debug_bounds(if side == "local-file" {
                    "local-file-1"
                } else {
                    "remote-file-1"
                })
                .unwrap();
            cx.simulate_click(first.center(), Modifiers::default());
            cx.simulate_click(second.center(), command);
            let view = view_entity.as_ref().unwrap();
            view.read_with(cx, |view, _| {
                if side == "local-file" {
                    assert_eq!(view.local_selected_paths.len(), 2);
                    assert_eq!(view.local_selected_paths[1].file_name().unwrap(), expected);
                } else {
                    assert_eq!(view.remote_selected_names.len(), 2);
                    assert_eq!(view.remote_selected_names[1], expected);
                }
            });
        }
        cx.simulate_keystrokes("ctrl-a");
        view_entity.as_ref().unwrap().read_with(cx, |view, _| {
            assert_eq!(view.remote_selected_names.len(), 500);
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let local = cx.debug_bounds("local-file-0").unwrap();
        cx.simulate_click(local.center(), Modifiers::default());
        cx.simulate_keystrokes("cmd-a");
        view_entity.as_ref().unwrap().read_with(cx, |view, _| {
            assert_eq!(view.local_selected_paths.len(), 500);
        });
    }

    #[gpui::test]
    fn photos_pane_renders_at_minimum_size(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("first.jpg"), b"first").unwrap();
        std::fs::write(directory.path().join("second.jpg"), b"second").unwrap();
        let remote = FileTransferSnapshot {
            can_photos: true,
            can_photo_delete: true,
            can_photo_batch_delete: true,
            photo_total: 56,
            photo_entries: (0..PHOTO_PAGE_SIZE)
                .map(|index| PhotoEntry {
                    id: format!("photo-{index}"),
                    name: format!("IMG_{index:04}.HEIC"),
                    created: 1_720_000_000,
                    width: 4032,
                    height: 3024,
                    thumbnail: Vec::new(),
                })
                .collect(),
            ..Default::default()
        };
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::bind_keys(cx);
        });
        let mut view_entity = None;
        let (_, cx) = cx.add_window_view(|window, cx| {
            let focus = cx.focus_handle();
            focus.focus(window, cx);
            let local_path_input = cx.new(|cx| InputState::new(window, cx));
            let remote_path_input = cx.new(|cx| InputState::new(window, cx));
            let new_folder_input = cx.new(|cx| InputState::new(window, cx));
            let view = cx.new(|_| FileTransferView {
                host: "example.test:5901".into(),
                local_dir: directory.path().to_path_buf(),
                local_path_input,
                remote_path_input,
                local_back: Vec::new(),
                remote_back: Vec::new(),
                local_entries: read_local_dir(directory.path()).unwrap(),
                local_selected: None,
                local_selected_paths: Vec::new(),
                local_anchor: None,
                local_sort: FileSort::default(),
                local_loading: false,
                local_error: None,
                transfers_open: true,
                queue: Vec::new(),
                current_job: None,
                pending_conflicts: None,
                pending_batch_delete: None,
                queue_idle_since: None,
                auto_collapse_armed: false,
                client: None,
                remote,
                preferred_remote: None,
                memory: test_memory(),
                remote_selected: None,
                remote_selected_names: Vec::new(),
                remote_anchor: None,
                remote_sort: FileSort::default(),
                active_side: FileSide::Remote,
                photo_mode: true,
                photo_selected: Some("photo-0".into()),
                photo_selected_ids: vec!["photo-0".into(), "photo-1".into()],
                photo_anchor: None,
                photo_thumbs: HashMap::new(),
                allow_upload: true,
                details: None,
                pending_delete: None,
                pending_rename: None,
                new_folder_side: None,
                new_folder_input,
                status_error: None,
                fullscreen_toolbar_open: false,
                focus,
                _subscriptions: Vec::new(),
            });
            view_entity = Some(view.clone());
            gpui_component::Root::new(view, window, cx)
        });
        cx.simulate_resize(size(px(760.), px(440.)));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let first = cx.debug_bounds("photo-0").unwrap();
        let third = cx.debug_bounds("photo-2").unwrap();
        cx.simulate_click(first.center(), Modifiers::default());
        cx.simulate_click(
            third.center(),
            Modifiers {
                platform: true,
                ..Default::default()
            },
        );
        let view_entity = view_entity.unwrap();
        view_entity.read_with(cx, |view, _| {
            assert_eq!(view.photo_selected_ids, ["photo-0", "photo-2"]);
        });
        cx.simulate_keystrokes("ctrl-a");
        view_entity.read_with(cx, |view, _| {
            assert_eq!(view.photo_selected_ids.len(), PHOTO_PAGE_SIZE);
        });

        let mut thumbnail = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            2,
            2,
            image::Rgba([231, 73, 17, 255]),
        ))
        .write_to(&mut thumbnail, image::ImageFormat::Png)
        .unwrap();
        view_entity.update(cx, |view, cx| {
            view.remote.photo_entries[0].thumbnail = thumbnail.into_inner();
            view.reconcile_photo_thumbs(cx);
            assert!(view.photo_thumbs.contains_key("photo-0"));
            assert_eq!(
                &view.photo_thumbs["photo-0"].as_bytes(0).unwrap()[..4],
                &[17, 73, 231, 255],
                "Photos thumbnails must use GPUI's BGRA channel order",
            );

            view.remote.photo_entries[0].thumbnail.clear();
            view.reconcile_photo_thumbs(cx);
            assert!(view.photo_thumbs.contains_key("photo-0"));

            view.remote.photo_entries.remove(0);
            view.reconcile_photo_thumbs(cx);
            assert!(!view.photo_thumbs.contains_key("photo-0"));
        });

        std::fs::write(directory.path().join("IMG_0002.HEIC"), b"existing").unwrap();
        view_entity.update(cx, |view, cx| {
            view.photo_selected_ids = vec!["photo-1".into(), "photo-2".into()];
            view.photo_selected = Some("photo-2".into());
            view.download_photo(cx);
            assert_eq!(view.queue.len(), 2);
            assert!(matches!(view.queue[0].state, QueueState::Running));
            assert!(matches!(view.queue[1].state, QueueState::Queued));
            assert_eq!(view.queue[1].name, "IMG_0002.HEIC");
            let QueueAction::PhotoExport {
                destination: first_stage,
                ..
            } = &view.queue[0].action
            else {
                panic!("expected photo export");
            };
            std::fs::write(first_stage, b"first photo").unwrap();
            let QueueAction::PhotoExport {
                destination: second_stage,
                replace: Some(second_target),
                ..
            } = &view.queue[1].action
            else {
                panic!("expected staged photo replacement");
            };
            let second_stage = second_stage.clone();
            let second_target = second_target.clone();
            assert_ne!(second_stage, second_target);
            assert_eq!(second_target, directory.path().join("IMG_0002.HEIC"));
            assert_eq!(
                std::fs::read(directory.path().join("IMG_0002.HEIC")).unwrap(),
                b"existing"
            );

            view.remote.photo_status = Some("Original photo downloaded".into());
            view.remote.photo_revision += 1;
            view.advance_queue(cx);
            assert!(matches!(view.queue[0].state, QueueState::Complete));
            assert!(matches!(view.queue[1].state, QueueState::Running));

            std::fs::write(&second_stage, b"new photo").unwrap();
            view.remote.photo_revision += 1;
            view.advance_queue(cx);
            assert!(matches!(view.queue[1].state, QueueState::Complete));
            assert_eq!(
                std::fs::read(directory.path().join("IMG_0002.HEIC")).unwrap(),
                b"new photo"
            );

            view.local_selected_paths = vec![
                directory.path().join("first.jpg"),
                directory.path().join("second.jpg"),
            ];
            view.local_selected = view.local_selected_paths.last().cloned();
            view.upload_to_photos(cx);
            assert_eq!(view.queue.len(), 4);
            assert!(matches!(view.queue[2].state, QueueState::Running));
            assert!(matches!(view.queue[3].state, QueueState::Queued));
            assert!(matches!(view.queue[2].action, QueueAction::PhotoUpload(_)));

            view.remote.photo_status = Some("Imported into Photos · first".into());
            view.remote.photo_revision += 1;
            view.remote.photo_busy = true;
            view.advance_queue(cx);
            assert!(matches!(view.queue[2].state, QueueState::Running));

            view.remote.photo_revision += 1;
            view.remote.photo_busy = false;
            view.remote.photos_need_refresh = true;
            view.advance_queue(cx);
            assert!(matches!(view.queue[2].state, QueueState::Complete));
            assert!(
                view.remote.photos_need_refresh,
                "do not refresh between queued images"
            );
            assert!(matches!(view.queue[3].state, QueueState::Running));

            view.remote.photo_status = Some("Imported into Photos · second".into());
            view.remote.photo_revision += 1;
            view.advance_queue(cx);
            assert!(matches!(view.queue[3].state, QueueState::Complete));
            assert!(
                view.remote.photo_busy,
                "refresh once at the end of the batch"
            );
            assert!(!view.remote.photos_need_refresh);
            // A failed refresh must not trigger another automatic attempt.
            view.remote.photo_busy = false;
            view.remote.photo_error = Some("Photos refresh failed".into());
            view.advance_queue(cx);
            assert!(!view.remote.photo_busy);
            assert_eq!(
                view.remote.photo_error.as_deref(),
                Some("Photos refresh failed")
            );
        });
    }
}
