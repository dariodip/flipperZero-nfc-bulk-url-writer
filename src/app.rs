//! Application controller: wires the UI, storage and NFC worker to the core
//! workflow.

use core::ffi::c_void;
use core::fmt::Write as FmtWrite;
use core::sync::atomic::{AtomicBool, Ordering};

use flipperzero::furi::string::FuriString;
use flipperzero_sys as sys;
use nfc_bulk_url_writer_core::MAX_URL_LEN;
use nfc_bulk_url_writer_core::errors::{PlaylistError, SessionError, TagError};
use nfc_bulk_url_writer_core::name::FixedName;
use nfc_bulk_url_writer_core::ndef;
use nfc_bulk_url_writer_core::playlist::PlaylistStats;
use nfc_bulk_url_writer_core::session::{MAX_PATH_LEN, ResumeStatus, Session, resume_status};
use nfc_bulk_url_writer_core::tag::MAX_UID_LEN;
use nfc_bulk_url_writer_core::workflow::{Action, Screen, Workflow, WorkflowErrorKind};

use crate::nfc::worker::{EVENT_NFC_RESULT, NfcRequest, NfcResult, NfcWorker};
use crate::storage::{self, FolderList, LoadedSession, PlaylistList, PlaylistLoadError};
use crate::ui::Ui;

const DEBUG_FLAG: &[u8] = b"/ext/apps_data/nfc-bulk-url-writer/debug";
const AUTO_READ_FLAG: &[u8] = b"/ext/apps_data/nfc-bulk-url-writer/auto-read";
const AUTO_WRITE_FLAG: &[u8] = b"/ext/apps_data/nfc-bulk-url-writer/auto-write";
const AUTO_URL_PATH: &[u8] = b"/ext/apps_data/nfc-bulk-url-writer/auto-url.txt";
const EVENT_NFC_TIMEOUT: u32 = 0x101;
const EVENT_BACK_PENDING: u32 = 0x104;
const EVENT_AUTO_START: u32 = 0x102;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutoMode {
    None,
    Read,
    Write,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DialogMode {
    None,
    PlaylistConfirm,
    Success,
    Error,
    Summary,
    ConfirmAbort,
    ConfirmExit,
    NoPlaylists,
    PlaylistErrorBack,
    ExitOnly,
    Diagnostics,
    Preview,
}

pub struct App {
    ui: Ui,
    workflow: Workflow,
    pending_session: Option<Session>,
    playlist_list: PlaylistList,
    playlist_name: Option<FixedName>,
    playlist_stats: Option<PlaylistStats>,
    playlist_dir: [u8; MAX_PATH_LEN],
    playlist_dir_len: usize,
    folder_list: FolderList,
    folder_path: [u8; MAX_PATH_LEN],
    folder_path_len: usize,
    folder_mode: bool,
    playlist_path: [u8; MAX_PATH_LEN],
    playlist_path_len: usize,
    url: [u8; MAX_URL_LEN],
    url_len: usize,
    required_tlv: usize,
    tag_ndef_memory: u16,
    programmed_uid: [u8; MAX_UID_LEN],
    programmed_uid_len: u8,
    removal_started: bool,
    worker: Option<NfcWorker>,
    dialog_mode: DialogMode,
    startup_resume_available: bool,
    debug_enabled: bool,
    diagnostics: bool,
    preview_active: bool,
    preview_step: u8,
    timeout_timer: *mut sys::FuriTimer,
    diagnostics_page: u8,
    diagnostics_detail: FuriString,
    auto_mode: AutoMode,
    auto_timer: *mut sys::FuriTimer,
    input_pubsub: *mut sys::FuriPubSub,
    input_subscription: *mut sys::FuriPubSubSubscription,
    long_back: AtomicBool,
    back_timer: *mut sys::FuriTimer,
}

impl App {
    pub fn new() -> Self {
        Self {
            ui: Ui::new(),
            workflow: Workflow::new(),
            pending_session: None,
            playlist_list: PlaylistList::empty(),
            playlist_name: None,
            playlist_stats: None,
            playlist_dir: [0u8; MAX_PATH_LEN],
            playlist_dir_len: 0,
            folder_list: FolderList::empty(),
            folder_path: [0u8; MAX_PATH_LEN],
            folder_path_len: 0,
            folder_mode: false,
            playlist_path: [0u8; MAX_PATH_LEN],
            playlist_path_len: 0,
            url: [0u8; MAX_URL_LEN],
            url_len: 0,
            required_tlv: 0,
            tag_ndef_memory: 0,
            programmed_uid: [0u8; MAX_UID_LEN],
            programmed_uid_len: 0,
            removal_started: false,
            worker: None,
            dialog_mode: DialogMode::None,
            startup_resume_available: false,
            debug_enabled: false,
            diagnostics: false,
            preview_active: false,
            preview_step: 0,
            timeout_timer: core::ptr::null_mut(),
            diagnostics_page: 0,
            diagnostics_detail: FuriString::new(),
            auto_mode: AutoMode::None,
            auto_timer: core::ptr::null_mut(),
            input_pubsub: core::ptr::null_mut(),
            input_subscription: core::ptr::null_mut(),
            long_back: AtomicBool::new(false),
            back_timer: core::ptr::null_mut(),
        }
    }

    pub fn initialize(&mut self, gui: &flipperzero::gui::Gui) {
        self.ui.attach(gui);
        let context = self as *mut App as *mut c_void;
        self.ui.set_app_context(context);
        self.ui.set_event_context(context);
        self.ui
            .set_custom_event_callback(Some(custom_event_callback));
        self.ui
            .set_navigation_event_callback(Some(navigation_callback));
        self.ui
            .popup_set_timeout_callback(Some(preview_popup_callback));
        let input =
            unsafe { sys::furi_record_open(c"input_events".as_ptr()) } as *mut sys::FuriPubSub;
        if !input.is_null() {
            self.input_pubsub = input;
            self.input_subscription =
                unsafe { sys::furi_pubsub_subscribe(input, Some(input_event_callback), context) };
        }
        self.timeout_timer = unsafe {
            sys::furi_timer_alloc(
                Some(operation_timeout_callback),
                sys::FuriTimerTypeOnce,
                self.ui.dispatcher as *mut c_void,
            )
        };
        self.back_timer = unsafe {
            sys::furi_timer_alloc(
                Some(back_pending_callback),
                sys::FuriTimerTypeOnce,
                self.ui.dispatcher as *mut c_void,
            )
        };
        self.debug_enabled = storage::path_exists(DEBUG_FLAG);
        self.playlist_dir_len =
            storage::load_playlist_dir(storage::PLAYLIST_DIR.to_bytes(), &mut self.playlist_dir);
        if storage::path_exists(AUTO_READ_FLAG) {
            self.auto_mode = AutoMode::Read;
        } else if storage::path_exists(AUTO_WRITE_FLAG) {
            self.auto_mode = AutoMode::Write;
        }
        if self.auto_mode != AutoMode::None {
            self.auto_timer = unsafe {
                sys::furi_timer_alloc(
                    Some(auto_start_callback),
                    sys::FuriTimerTypeOnce,
                    self.ui.dispatcher as *mut c_void,
                )
            };
            if !self.auto_timer.is_null() {
                unsafe {
                    sys::furi_timer_start(self.auto_timer, sys::furi_ms_to_ticks(300));
                }
            }
        } else {
            self.start_session();
        }
    }

    pub fn dispatcher(&self) -> *mut sys::ViewDispatcher {
        self.ui.dispatcher
    }

    pub fn shutdown(&mut self) {
        self.stop_worker();
        if !self.timeout_timer.is_null() {
            unsafe {
                sys::furi_timer_stop(self.timeout_timer);
                sys::furi_timer_free(self.timeout_timer);
            }
            self.timeout_timer = core::ptr::null_mut();
        }
        if !self.auto_timer.is_null() {
            unsafe {
                sys::furi_timer_stop(self.auto_timer);
                sys::furi_timer_free(self.auto_timer);
            }
            self.auto_timer = core::ptr::null_mut();
        }
        if !self.back_timer.is_null() {
            unsafe {
                sys::furi_timer_stop(self.back_timer);
                sys::furi_timer_free(self.back_timer);
            }
            self.back_timer = core::ptr::null_mut();
        }
        if !self.input_subscription.is_null() {
            unsafe { sys::furi_pubsub_unsubscribe(self.input_pubsub, self.input_subscription) };
            self.input_subscription = core::ptr::null_mut();
        }
        if !self.input_pubsub.is_null() {
            unsafe { sys::furi_record_close(c"input_events".as_ptr()) };
            self.input_pubsub = core::ptr::null_mut();
        }
        self.ui.free();
    }

    fn auto_line(args: core::fmt::Arguments) {
        let mut text = FuriString::new();
        let _ = text.write_fmt(args);
        storage::trace(text.as_c_str().to_str().unwrap_or("AUTO ?"));
        let mut out = flipperzero::furi::io::Stdout;
        let _ = out.write_fmt(args);
        let _ = out.write_str("\n");
        let _ = out.flush();
    }

    fn start_auto_read(&mut self) {
        self.diagnostics = true;
        self.dialog_mode = DialogMode::None;
        self.ui.show_waiting(1, 1, b"");
        if !self.ensure_worker() {
            Self::auto_line(format_args!("AUTO-READ error=worker"));
            self.exit();
            return;
        }
        let _ = self.worker.as_ref().map(|worker| {
            worker.send(NfcRequest::ReadAndValidate {
                required_tlv: 0,
                full_dump: true,
            })
        });
    }

    fn start_auto_write(&mut self) {
        let mut buffer = [0u8; MAX_URL_LEN];
        let read = storage::read_file_into(AUTO_URL_PATH, &mut buffer);
        let length = match read {
            Ok(length) => length,
            Err(_) => {
                Self::auto_line(format_args!("AUTO-WRITE error=url-file"));
                self.exit();
                return;
            }
        };
        let mut length = length;
        while length > 0 && (buffer[length - 1] == b'\n' || buffer[length - 1] == b'\r') {
            length -= 1;
        }
        self.url[..length].copy_from_slice(&buffer[..length]);
        self.url_len = length;
        match ndef::encoded_size(&buffer[..length]) {
            Ok(required) => self.required_tlv = required,
            Err(error) => {
                Self::auto_line(format_args!("AUTO-WRITE error={error}"));
                self.exit();
                return;
            }
        }
        self.ui.show_waiting(1, 1, &self.url[..length]);
        if !self.ensure_worker() {
            Self::auto_line(format_args!("AUTO-WRITE error=worker"));
            self.exit();
            return;
        }
        let _ = self.worker.as_ref().map(|worker| {
            worker.send(NfcRequest::ReadAndValidate {
                required_tlv: self.required_tlv as u16,
                full_dump: false,
            })
        });
    }

    fn handle_auto_result(&mut self, result: NfcResult) {
        match self.auto_mode {
            AutoMode::None => {}
            AutoMode::Read => {
                match result {
                    NfcResult::Validated {
                        model,
                        ndef_memory,
                        uid,
                        uid_len,
                        version,
                        pages_read,
                        cc,
                        static_lock,
                        dynamic_lock,
                        config,
                    } => {
                        let mut report = FuriString::new();
                        let _ = write!(
                            report,
                            "AUTO-READ ok model={} pages={} read={} uid=",
                            model.name(),
                            model.page_count(),
                            pages_read
                        );
                        push_hex(&mut report, &uid[..uid_len as usize]);
                        let _ = write!(report, " version=");
                        push_hex(&mut report, &version);
                        let _ = write!(
                            report,
                            " cc={:02X}{:02X}{:02X}{:02X} static={:02X}{:02X} dyn={:02X}{:02X}{:02X}{:02X} cfg={:02X}{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}{:02X} ndef={}",
                            cc[0],
                            cc[1],
                            cc[2],
                            cc[3],
                            static_lock[0],
                            static_lock[1],
                            dynamic_lock[0],
                            dynamic_lock[1],
                            dynamic_lock[2],
                            dynamic_lock[3],
                            config[0],
                            config[1],
                            config[2],
                            config[3],
                            config[4],
                            config[5],
                            config[6],
                            config[7],
                            ndef_memory
                        );
                        Self::auto_line(format_args!(
                            "{}",
                            report.as_c_str().to_str().unwrap_or("?")
                        ));
                    }
                    NfcResult::Rejected(error) => {
                        Self::auto_line(format_args!("AUTO-READ error={error}"));
                    }
                    _ => Self::auto_line(format_args!("AUTO-READ error=unexpected")),
                }
                self.exit();
            }
            AutoMode::Write => match result {
                NfcResult::Validated {
                    model,
                    uid,
                    uid_len,
                    ..
                } => {
                    self.programmed_uid = uid;
                    self.programmed_uid_len = uid_len;
                    let mut report = FuriString::new();
                    let _ = write!(report, "AUTO-WRITE read-ok model={} uid=", model.name());
                    push_hex(&mut report, &uid[..uid_len as usize]);
                    Self::auto_line(format_args!(
                        "{}",
                        report.as_c_str().to_str().unwrap_or("?")
                    ));
                    if let Some(worker) = self.worker.as_ref() {
                        let _ = worker.send(NfcRequest::WriteAndVerify {
                            required_tlv: self.required_tlv as u16,
                            url_len: self.url_len as u16,
                            url: self.url,
                        });
                    } else {
                        Self::auto_line(format_args!("AUTO-WRITE error=worker"));
                        self.exit();
                    }
                }
                NfcResult::Verified => {
                    Self::auto_line(format_args!(
                        "AUTO-WRITE verified url={}",
                        core::str::from_utf8(&self.url[..self.url_len]).unwrap_or("?")
                    ));
                    self.exit();
                }
                NfcResult::NdefRejected(error) => {
                    Self::auto_line(format_args!("AUTO-WRITE error={error}"));
                    self.exit();
                }
                NfcResult::Rejected(error) => {
                    Self::auto_line(format_args!("AUTO-WRITE error={error}"));
                    self.exit();
                }
                NfcResult::TagChanged => {
                    Self::auto_line(format_args!("AUTO-WRITE error=tag-changed"));
                    self.exit();
                }
                NfcResult::WriteFailed => {
                    Self::auto_line(format_args!("AUTO-WRITE error=write-failed"));
                    self.exit();
                }
                NfcResult::ReadbackFailed => {
                    Self::auto_line(format_args!("AUTO-WRITE error=readback-failed"));
                    self.exit();
                }
                NfcResult::VerificationFailed => {
                    Self::auto_line(format_args!("AUTO-WRITE error=verification-failed"));
                    self.exit();
                }
                NfcResult::SameTagPresent => {
                    Self::auto_line(format_args!("AUTO-WRITE error=same-tag"));
                    self.exit();
                }
                NfcResult::TagCleared => {
                    Self::auto_line(format_args!("AUTO-WRITE error=tag-cleared"));
                    self.exit();
                }
                NfcResult::InternalError => {
                    Self::auto_line(format_args!("AUTO-WRITE error=internal"));
                    self.exit();
                }
            },
        }
    }

    fn start_session(&mut self) {
        let mut loaded = match Session::new(0, 1, b"/") {
            Ok(session) => session,
            Err(error) => {
                self.render_error(WorkflowErrorKind::Session(error));
                return;
            }
        };

        match storage::load_session(&mut loaded) {
            LoadedSession::Missing => {
                self.pending_session = None;
                self.startup_resume_available = false;
                let _ = self.workflow.start(None, ResumeStatus::NoSession);
                self.ui.show_startup(false, self.debug_enabled);
            }
            LoadedSession::Corrupt => {
                self.render_error(WorkflowErrorKind::Session(SessionError::Corrupt));
            }
            LoadedSession::ReadError => {
                self.render_error(WorkflowErrorKind::Storage);
            }
            LoadedSession::Loaded => {
                if loaded.is_complete() {
                    let action = self.workflow.start(Some(loaded), ResumeStatus::Completed);
                    self.render(action);
                    return;
                }

                self.copy_path(loaded.path());
                self.playlist_name = storage::display_name_from_path(loaded.path());
                match storage::scan_playlist_path(loaded.path()) {
                    Ok(stats) => {
                        match resume_status(Some(&loaded), true, stats.fingerprint, stats.total) {
                            ResumeStatus::Resume => {
                                self.pending_session = Some(loaded);
                                self.startup_resume_available = true;
                                self.ui.show_startup(true, self.debug_enabled);
                            }
                            ResumeStatus::Completed => {
                                let action =
                                    self.workflow.start(Some(loaded), ResumeStatus::Completed);
                                self.render(action);
                            }
                            ResumeStatus::PlaylistChanged => {
                                let action = self
                                    .workflow
                                    .start(Some(loaded), ResumeStatus::PlaylistChanged);
                                self.render(action);
                            }
                            ResumeStatus::PlaylistMissing => {
                                let action = self
                                    .workflow
                                    .start(Some(loaded), ResumeStatus::PlaylistMissing);
                                self.render(action);
                            }
                            ResumeStatus::NoSession => {
                                self.ui.show_startup(false, self.debug_enabled);
                            }
                        }
                    }
                    Err(PlaylistLoadError::Missing) => {
                        let action = self
                            .workflow
                            .start(Some(loaded), ResumeStatus::PlaylistMissing);
                        self.render(action);
                    }
                    Err(PlaylistLoadError::Read) => {
                        self.render_error(WorkflowErrorKind::Storage);
                    }
                    Err(PlaylistLoadError::Parse(_)) => {
                        let action = self
                            .workflow
                            .start(Some(loaded), ResumeStatus::PlaylistChanged);
                        self.render(action);
                    }
                }
            }
        }
    }

    fn on_startup_item(&mut self, index: u32) {
        if self.startup_resume_available {
            match index {
                0 => self.resume_session(),
                1 => self.begin_new(),
                2 => self.start_folder_picker(),
                3 => self.start_diagnostics(),
                4 => self.begin_preview(),
                _ => {}
            }
        } else {
            match index {
                0 => self.begin_new(),
                1 => self.start_folder_picker(),
                2 => self.start_diagnostics(),
                3 => self.begin_preview(),
                _ => {}
            }
        }
    }

    fn resume_session(&mut self) {
        let Some(session) = self.pending_session.take() else {
            self.begin_new();
            return;
        };
        self.copy_path(session.path());
        self.playlist_name = storage::display_name_from_path(session.path());
        let action = self.workflow.start(Some(session), ResumeStatus::Resume);
        self.render(action);
    }

    fn begin_new(&mut self) {
        self.stop_worker();
        self.pending_session = None;
        self.startup_resume_available = false;
        self.playlist_name = None;
        self.playlist_stats = None;
        self.diagnostics = false;
        self.preview_active = false;
        let _ = self.workflow.start(None, ResumeStatus::NoSession);
        self.show_playlist_choices();
    }

    fn show_playlist_choices(&mut self) {
        match storage::discover_playlists_in(&self.playlist_dir[..self.playlist_dir_len]) {
            Ok(list) => {
                if list.is_empty() {
                    let mut text = FuriString::new();
                    text.push_str("No .txt files in:\n");
                    push_path_text(&mut text, &self.playlist_dir[..self.playlist_dir_len]);
                    self.dialog_mode = DialogMode::NoPlaylists;
                    self.ui
                        .show_message_dynamic(c"No playlists", &text, c"Back");
                } else {
                    self.playlist_list = list;
                    self.ui.show_playlists(&self.playlist_list);
                }
            }
            Err(storage::StorageError::MissingDirectory) => {
                let mut text = FuriString::new();
                text.push_str("Folder not found:\n");
                push_path_text(&mut text, &self.playlist_dir[..self.playlist_dir_len]);
                self.dialog_mode = DialogMode::NoPlaylists;
                self.ui
                    .show_message_dynamic(c"Playlist folder", &text, c"Back");
            }
            Err(_) => {
                self.render_error(WorkflowErrorKind::Storage);
            }
        }
    }

    fn start_folder_picker(&mut self) {
        self.folder_mode = true;
        let mut start = [0u8; MAX_PATH_LEN];
        let length = if self.playlist_dir_len > 0 {
            let length = self.playlist_dir_len.min(MAX_PATH_LEN);
            start[..length].copy_from_slice(&self.playlist_dir[..length]);
            length
        } else {
            start[..4].copy_from_slice(b"/ext");
            4
        };
        self.set_folder_path(&start[..length]);
        self.refresh_folder();
    }

    fn set_folder_path(&mut self, path: &[u8]) {
        let length = path.len().min(MAX_PATH_LEN);
        self.folder_path[..length].copy_from_slice(&path[..length]);
        self.folder_path_len = length;
    }

    fn refresh_folder(&mut self) {
        match storage::list_subdirectories(&self.folder_path[..self.folder_path_len]) {
            Ok(list) => {
                self.folder_list = list;
                let path = &self.folder_path[..self.folder_path_len];
                let header = path
                    .iter()
                    .rposition(|&byte| byte == b'/')
                    .map(|index| &path[index + 1..])
                    .unwrap_or(path);
                let header = if header.is_empty() {
                    b"ext" as &[u8]
                } else {
                    header
                };
                let has_parent = storage::parent_path(path).is_some();
                self.ui.show_folder(header, &self.folder_list, has_parent);
            }
            Err(_) => {
                self.folder_mode = false;
                self.dialog_mode = DialogMode::NoPlaylists;
                self.ui
                    .show_message(c"Folder unavailable", c"Choose another folder.", c"Back");
            }
        }
    }

    fn on_folder_item(&mut self, index: u32) {
        if index == crate::ui::FOLDER_USE {
            let _ = storage::save_playlist_dir(&self.folder_path[..self.folder_path_len]);
            let length = self.folder_path_len;
            self.playlist_dir[..length].copy_from_slice(&self.folder_path[..length]);
            self.playlist_dir_len = length;
            self.folder_mode = false;
            self.show_playlist_choices();
        } else if index == crate::ui::FOLDER_PARENT {
            if let Some(parent) = storage::parent_path(&self.folder_path[..self.folder_path_len]) {
                let mut buffer = [0u8; MAX_PATH_LEN];
                let length = parent.len().min(MAX_PATH_LEN);
                buffer[..length].copy_from_slice(&parent[..length]);
                self.set_folder_path(&buffer[..length]);
                self.refresh_folder();
            }
        } else if let Some(name) = self.folder_list.get(index as usize).copied() {
            let mut buffer = [0u8; MAX_PATH_LEN];
            if let Some(length) = storage::join_path(
                &self.folder_path[..self.folder_path_len],
                name.as_bytes(),
                &mut buffer,
            ) {
                self.set_folder_path(&buffer[..length]);
                self.refresh_folder();
            } else {
                self.folder_mode = false;
                self.dialog_mode = DialogMode::NoPlaylists;
                self.ui
                    .show_message(c"Path too long", c"Choose another folder.", c"Back");
            }
        }
    }

    fn on_playlist_item(&mut self, index: u32) {
        let Some(name) = self.playlist_list.get(index as usize).copied() else {
            return;
        };
        let path = storage::playlist_path_in(&self.playlist_dir[..self.playlist_dir_len], &name);
        let path_bytes = path.as_c_str().to_bytes();
        match storage::scan_playlist_path(path_bytes) {
            Ok(stats) => {
                self.playlist_name = Some(name);
                self.playlist_stats = Some(stats);
                self.dialog_mode = DialogMode::PlaylistConfirm;
                self.ui.show_playlist_confirm(&name, &stats);
            }
            Err(PlaylistLoadError::Missing) => {
                self.dialog_mode = DialogMode::PlaylistErrorBack;
                self.ui.show_message(
                    c"Playlist missing",
                    c"The selected file no\nlonger exists.",
                    c"Back",
                );
            }
            Err(PlaylistLoadError::Parse(error)) => {
                let mut text = FuriString::new();
                let _ = write!(text, "{error}");
                self.dialog_mode = DialogMode::PlaylistErrorBack;
                self.ui
                    .show_message_dynamic(c"Invalid playlist", &text, c"Back");
            }
            Err(PlaylistLoadError::Read) => {
                self.render_error(WorkflowErrorKind::Storage);
            }
        }
    }

    fn confirm_playlist(&mut self) {
        let Some(name) = self.playlist_name else {
            return;
        };
        let Some(stats) = self.playlist_stats else {
            return;
        };
        let path = storage::playlist_path_in(&self.playlist_dir[..self.playlist_dir_len], &name);
        let bytes = path.as_c_str().to_bytes();
        self.copy_path(bytes);
        // Starting a new batch replaces any previous session. If the new
        // session is aborted before its first save, the old progress is gone,
        // which is what "Start new" promises.
        let _ = storage::clear_session();
        match Session::new(stats.fingerprint, stats.total, bytes) {
            Ok(session) => {
                let action = self.workflow.playlist_loaded(session);
                self.render(action);
            }
            Err(error) => {
                self.render_error(WorkflowErrorKind::Session(error));
            }
        }
    }

    fn begin_preview(&mut self) {
        self.diagnostics = false;
        self.preview_active = true;
        self.preview_step = 0;
        self.show_preview();
    }

    fn show_preview(&mut self) {
        match self.preview_step {
            0 => {
                self.dialog_mode = DialogMode::Preview;
                self.ui
                    .show_waiting(3, 42, b"https://example.test/contact/anna");
                self.ui
                    .popup_set_timeout_callback(Some(preview_popup_callback));
                self.ui.enable_popup_timeout(1200);
            }
            1 => {
                self.dialog_mode = DialogMode::Preview;
                self.ui
                    .show_writing(3, 42, Some("NTAG213"), Some(144), false);
                self.ui
                    .popup_set_timeout_callback(Some(preview_popup_callback));
                self.ui.enable_popup_timeout(1200);
            }
            2 => {
                self.dialog_mode = DialogMode::Preview;
                self.ui.show_success(3, 42, "NTAG213");
            }
            3 => {
                self.dialog_mode = DialogMode::Preview;
                self.ui.show_remove_tag(false);
            }
            4 => {
                self.dialog_mode = DialogMode::Preview;
                self.ui.show_error(WorkflowErrorKind::Tag(TagError::Locked));
            }
            5 => {
                self.dialog_mode = DialogMode::Preview;
                self.ui.show_summary(
                    nfc_bulk_url_writer_core::workflow::Summary {
                        total: 42,
                        completed: 38,
                        skipped: 2,
                        failed: 3,
                        remaining: 2,
                        complete: false,
                        aborted: true,
                    },
                    false,
                );
            }
            _ => {
                self.preview_active = false;
                self.dialog_mode = DialogMode::None;
                self.ui
                    .show_startup(self.startup_resume_available, self.debug_enabled);
            }
        }
    }

    fn advance_preview(&mut self) {
        self.preview_step = self.preview_step.saturating_add(1);
        self.show_preview();
    }

    fn start_diagnostics(&mut self) {
        self.diagnostics = true;
        self.preview_active = false;
        self.dialog_mode = DialogMode::None;
        self.ui.show_waiting(1, 1, b"");
        if self.ensure_worker() {
            let _ = self.worker.as_ref().map(|worker| {
                worker.send(NfcRequest::ReadAndValidate {
                    required_tlv: 0,
                    full_dump: true,
                })
            });
        }
    }

    fn enter_waiting(&mut self) {
        let Some(session) = self.workflow.session() else {
            self.render_error(WorkflowErrorKind::Session(SessionError::Corrupt));
            return;
        };
        let index = session.next;
        let total = session.total;

        let mut path = [0u8; MAX_PATH_LEN];
        let path_len = self.playlist_path_len.min(MAX_PATH_LEN);
        path[..path_len].copy_from_slice(&self.playlist_path[..path_len]);

        let read = storage::playlist_entry_path(&path[..path_len], index, &mut self.url);
        let length = match read {
            Ok(length) => length,
            Err(PlaylistLoadError::Missing) => {
                self.render_error(WorkflowErrorKind::Playlist(PlaylistError::Source));
                return;
            }
            Err(PlaylistLoadError::Read) => {
                self.render_error(WorkflowErrorKind::Storage);
                return;
            }
            Err(PlaylistLoadError::Parse(error)) => {
                self.render_error(WorkflowErrorKind::Playlist(error));
                return;
            }
        };
        self.url_len = length;

        match ndef::encoded_size(&self.url[..length]) {
            Ok(required) => self.required_tlv = required,
            Err(error) => {
                let action = self.workflow.ndef_rejected(error);
                self.render(action);
                return;
            }
        }

        self.ui.show_waiting(index + 1, total, &self.url[..length]);
        if self.ensure_worker() {
            let _ = self.worker.as_ref().map(|worker| {
                worker.send(NfcRequest::ReadAndValidate {
                    required_tlv: self.required_tlv as u16,
                    full_dump: false,
                })
            });
        }
    }

    fn arm_operation_timeout(&self) {
        if !self.timeout_timer.is_null() {
            unsafe {
                sys::furi_timer_stop(self.timeout_timer);
                sys::furi_timer_start(self.timeout_timer, sys::furi_ms_to_ticks(30_000));
            }
        }
    }

    fn disarm_operation_timeout(&self) {
        if !self.timeout_timer.is_null() {
            unsafe { sys::furi_timer_stop(self.timeout_timer) };
        }
    }

    fn show_writing(&mut self) {
        let (index, total) = self.position();
        let model = self.workflow.tag_model();
        self.ui.show_writing(
            index,
            total,
            model.map(|model| model.name()),
            Some(self.tag_ndef_memory),
            false,
        );
        if self.ensure_worker() {
            self.arm_operation_timeout();
            let _ = self.worker.as_ref().map(|worker| {
                worker.send(NfcRequest::WriteAndVerify {
                    required_tlv: self.required_tlv as u16,
                    url_len: self.url_len as u16,
                    url: self.url,
                })
            });
        }
    }

    fn show_success(&mut self) {
        let (index, total) = self.position();
        let model = self
            .workflow
            .tag_model()
            .map(|model| model.name())
            .unwrap_or("Tag");
        self.dialog_mode = DialogMode::Success;
        self.ui.show_success(index, total, model);
    }

    fn show_remove(&mut self, same_tag: bool) {
        self.dialog_mode = DialogMode::None;
        if !same_tag && !self.removal_started {
            self.removal_started = true;
            if !self.ensure_worker() {
                return;
            }
            let _ = self.worker.as_ref().map(|worker| {
                worker.send(NfcRequest::CheckRemoval {
                    uid: self.programmed_uid,
                    uid_len: self.programmed_uid_len,
                })
            });
        }
        self.ui.show_remove_tag(same_tag);
    }

    fn position(&self) -> (u32, u32) {
        let session = self.workflow.session();
        (
            session
                .map(|session| (session.next + 1).min(session.total))
                .unwrap_or(1),
            session.map(|session| session.total).unwrap_or(0),
        )
    }

    fn handle_nfc_result(&mut self, result: NfcResult) {
        if self.auto_mode != AutoMode::None {
            self.handle_auto_result(result);
            return;
        }
        if self.diagnostics {
            match result {
                NfcResult::Validated {
                    model,
                    ndef_memory,
                    uid,
                    uid_len,
                    version,
                    pages_read,
                    cc,
                    static_lock,
                    dynamic_lock,
                    config,
                } => {
                    let mut text = FuriString::new();
                    let _ = write!(
                        text,
                        "{} {}p, {} read\nUID ",
                        model.name(),
                        model.page_count(),
                        pages_read
                    );
                    push_hex(&mut text, &uid[..uid_len as usize]);
                    let _ = write!(
                        text,
                        "\nCC {:02X} {:02X} {:02X} {:02X}\nV {:02X} {:02X} {}B",
                        cc[0], cc[1], cc[2], cc[3], version[6], version[7], ndef_memory
                    );
                    self.diagnostics_page = 0;
                    self.diagnostics_detail.clear();
                    let _ = write!(
                        self.diagnostics_detail,
                        "Static lock {:02X}{:02X}\nDyn lock {:02X}{:02X}{:02X}{:02X}\nCfg {:02X}{:02X}{:02X}{:02X}\n    {:02X}{:02X}{:02X}{:02X}",
                        static_lock[0],
                        static_lock[1],
                        dynamic_lock[0],
                        dynamic_lock[1],
                        dynamic_lock[2],
                        dynamic_lock[3],
                        config[0],
                        config[1],
                        config[2],
                        config[3],
                        config[4],
                        config[5],
                        config[6],
                        config[7]
                    );
                    self.dialog_mode = DialogMode::Diagnostics;
                    self.ui.show_message_dynamic(c"Diagnostics", &text, c"Next");
                }
                NfcResult::Rejected(error) => {
                    let mut text = FuriString::new();
                    let _ = write!(text, "{error}");
                    self.dialog_mode = DialogMode::ExitOnly;
                    self.ui.show_message_dynamic(c"Diagnostics", &text, c"Exit");
                }
                _ => {
                    self.dialog_mode = DialogMode::ExitOnly;
                    self.ui
                        .show_message(c"Diagnostics", c"No tag data.", c"Exit");
                }
            }
            return;
        }

        let action = match result {
            NfcResult::Validated {
                model,
                ndef_memory,
                uid,
                uid_len,
                ..
            } => {
                self.tag_ndef_memory = ndef_memory;
                self.programmed_uid = uid;
                self.programmed_uid_len = uid_len;
                self.workflow.tag_validated(model)
            }
            NfcResult::NdefRejected(error) => self.workflow.ndef_rejected(error),
            NfcResult::Rejected(error) => self.workflow.tag_rejected(error),
            NfcResult::TagChanged => self.workflow.tag_changed(),
            NfcResult::WriteFailed => self.workflow.write_failed(),
            NfcResult::ReadbackFailed => self.workflow.readback_failed(),
            NfcResult::VerificationFailed => self.workflow.verification_failed(),
            NfcResult::Verified => self.workflow.verification_succeeded(),
            NfcResult::SameTagPresent => self.workflow.same_tag_detected(),
            NfcResult::TagCleared => {
                self.removal_started = false;
                self.workflow.tag_cleared()
            }
            NfcResult::InternalError => self.workflow.tag_rejected(TagError::UnsupportedTag),
        };
        self.render(action);
    }

    fn on_dialog(&mut self, result: sys::DialogExResult) {
        let left = result == sys::DialogExResultLeft;
        let center = result == sys::DialogExResultCenter;
        let right = result == sys::DialogExResultRight;

        match self.dialog_mode {
            DialogMode::PlaylistConfirm => {
                if center {
                    self.confirm_playlist();
                } else if left {
                    self.show_playlist_choices();
                }
            }
            DialogMode::Success => {
                if center {
                    self.removal_started = false;
                    match self.workflow.confirm_success() {
                        Ok(action) => self.render(action),
                        Err(error) => self.render_error(WorkflowErrorKind::Session(error)),
                    }
                }
            }
            DialogMode::Error => {
                let tag_error = self
                    .workflow
                    .error()
                    .map(|error| error.is_tag_error())
                    .unwrap_or(false);
                if tag_error {
                    if left {
                        let action = self.workflow.retry();
                        self.render(action);
                    } else if center {
                        match self.workflow.skip() {
                            Ok(action) => self.render(action),
                            Err(error) => self.render_error(WorkflowErrorKind::Session(error)),
                        }
                    } else if right {
                        let action = self.workflow.abort();
                        self.render(action);
                    }
                } else if left {
                    self.begin_new();
                } else if center {
                    self.exit();
                }
            }
            DialogMode::Summary => {
                if left {
                    self.begin_new();
                } else if center {
                    self.restart_to_start();
                }
            }
            DialogMode::ConfirmAbort => {
                if center {
                    let action = self.workflow.abort();
                    self.render(action);
                } else {
                    self.redisplay();
                }
            }
            DialogMode::ConfirmExit => {
                if center {
                    self.exit();
                } else {
                    self.redisplay();
                }
            }
            DialogMode::NoPlaylists => {
                self.ui
                    .show_startup(self.startup_resume_available, self.debug_enabled);
            }
            DialogMode::ExitOnly => {
                self.exit();
            }
            DialogMode::PlaylistErrorBack => {
                self.show_playlist_choices();
            }
            DialogMode::Diagnostics => {
                if self.diagnostics_page == 0 {
                    self.diagnostics_page = 1;
                    self.ui
                        .show_message_dynamic(c"Diagnostics", &self.diagnostics_detail, c"Exit");
                } else {
                    self.diagnostics = false;
                    self.exit();
                }
            }
            DialogMode::Preview => {
                self.advance_preview();
            }
            DialogMode::None => {}
        }
    }

    fn show_exit_confirmation(&mut self) {
        self.dialog_mode = DialogMode::ConfirmExit;
        self.ui
            .show_confirmation(c"Exit app?", c"Close the application?", c"No", c"Yes");
    }

    fn on_back(&mut self) -> bool {
        if self.folder_mode {
            self.folder_mode = false;
            self.ui
                .show_startup(self.startup_resume_available, self.debug_enabled);
            return true;
        }
        match self.dialog_mode {
            DialogMode::PlaylistConfirm => {
                self.show_playlist_choices();
                return true;
            }
            DialogMode::PlaylistErrorBack => {
                self.show_playlist_choices();
                return true;
            }
            DialogMode::ConfirmAbort => {
                self.redisplay();
                return true;
            }
            DialogMode::ConfirmExit => {
                self.redisplay();
                return true;
            }
            DialogMode::NoPlaylists => {
                self.ui
                    .show_startup(self.startup_resume_available, self.debug_enabled);
                return true;
            }
            DialogMode::ExitOnly | DialogMode::Diagnostics => {
                self.exit();
                return true;
            }
            DialogMode::Summary => {
                self.restart_to_start();
                return true;
            }
            DialogMode::Preview => {
                self.advance_preview();
                return true;
            }
            DialogMode::Success => return true,
            DialogMode::Error => {
                let tag_error = self
                    .workflow
                    .error()
                    .map(|error| error.is_tag_error())
                    .unwrap_or(false);
                if !tag_error {
                    self.exit();
                }
                return true;
            }
            DialogMode::None => {}
        }

        match self.workflow.screen() {
            Screen::Startup | Screen::PlaylistSelect => {
                self.exit();
            }
            Screen::WaitingForTag | Screen::RemoveTag => {
                self.dialog_mode = DialogMode::ConfirmAbort;
                self.ui.show_confirmation(
                    c"Abort session?",
                    c"Current progress\nwill be saved.",
                    c"No",
                    c"Yes",
                );
            }
            Screen::Writing | Screen::Success => {}
            Screen::Error => {
                let tag_error = self
                    .workflow
                    .error()
                    .map(|error| error.is_tag_error())
                    .unwrap_or(false);
                if !tag_error {
                    self.exit();
                }
            }
            Screen::Summary => {
                let action = self.workflow.finish();
                self.render(action);
            }
            Screen::Finished => self.exit(),
        }
        true
    }

    fn redisplay(&mut self) {
        if self.folder_mode {
            self.refresh_folder();
            return;
        }
        if self.preview_active {
            self.show_preview();
            return;
        }
        if self.diagnostics {
            self.dialog_mode = DialogMode::None;
            self.ui.show_waiting(1, 1, b"");
            return;
        }
        match self.workflow.screen() {
            Screen::WaitingForTag => {
                let (index, total) = self.position();
                self.dialog_mode = DialogMode::None;
                self.ui
                    .show_waiting(index, total, &self.url[..self.url_len]);
            }
            Screen::Writing => {
                let (index, total) = self.position();
                let model = self.workflow.tag_model().map(|model| model.name());
                self.dialog_mode = DialogMode::None;
                self.ui
                    .show_writing(index, total, model, Some(self.tag_ndef_memory), false);
            }
            Screen::Success => self.show_success(),
            Screen::RemoveTag => {
                self.dialog_mode = DialogMode::None;
                self.ui.show_remove_tag(false);
            }
            Screen::Error => {
                if let Some(kind) = self.workflow.error() {
                    self.dialog_mode = DialogMode::Error;
                    self.ui.show_error(kind);
                }
            }
            Screen::Summary => {
                if let Some(summary) = self.workflow.summary() {
                    self.dialog_mode = DialogMode::Summary;
                    self.ui.show_summary(summary, summary.complete);
                }
            }
            Screen::Startup => {
                self.dialog_mode = DialogMode::None;
                self.ui
                    .show_startup(self.startup_resume_available, self.debug_enabled);
            }
            Screen::PlaylistSelect => {
                self.dialog_mode = DialogMode::None;
                self.show_playlist_choices();
            }
            Screen::Finished => {}
        }
    }

    fn render(&mut self, action: Action) {
        if !self.persist_if_dirty() {
            return;
        }
        match action {
            Action::ShowStartup { can_resume } => {
                self.ui.show_startup(can_resume, self.debug_enabled)
            }
            Action::ShowPlaylistSelect => self.show_playlist_choices(),
            Action::ShowWaiting => self.enter_waiting(),
            Action::ShowWriting => self.show_writing(),
            Action::ShowSuccess => self.show_success(),
            Action::ShowRemoveTag { same_tag } => self.show_remove(same_tag),
            Action::ShowError(kind) => self.render_error(kind),
            Action::ShowSummary(summary) => {
                self.stop_worker();
                self.dialog_mode = DialogMode::Summary;
                self.ui.show_summary(summary, summary.complete);
            }
            Action::Exit => self.exit(),
        }
    }

    fn render_error(&mut self, kind: WorkflowErrorKind) {
        self.stop_worker();
        self.dialog_mode = DialogMode::Error;
        self.ui.show_error(kind);
    }

    fn persist_if_dirty(&mut self) -> bool {
        if !self.workflow.take_dirty() {
            return true;
        }
        let Some(session) = self.workflow.session() else {
            return true;
        };
        if storage::save_session(session).is_err() {
            self.stop_worker();
            self.dialog_mode = DialogMode::ExitOnly;
            self.ui.show_message(
                c"Storage error",
                c"Progress was not saved.\nDo not remove the SD\ncard.",
                c"Exit",
            );
            return false;
        }
        true
    }

    fn ensure_worker(&mut self) -> bool {
        if self.worker.is_none() {
            let nfc = unsafe { sys::nfc_alloc() };
            if nfc.is_null() {
                return false;
            }
            self.worker = NfcWorker::start(self.ui.dispatcher, nfc);
            if self.worker.is_none() {
                unsafe { sys::nfc_free(nfc) };
            }
        }
        if self.worker.is_none() {
            self.dialog_mode = DialogMode::ExitOnly;
            self.ui
                .show_message(c"NFC error", c"Could not start the\nNFC worker.", c"Exit");
            return false;
        }
        true
    }

    fn stop_worker(&mut self) {
        if let Some(mut worker) = self.worker.take() {
            worker.stop();
        }
    }

    fn copy_path(&mut self, path: &[u8]) {
        let length = path.len().min(MAX_PATH_LEN);
        self.playlist_path[..length].copy_from_slice(&path[..length]);
        self.playlist_path_len = length;
    }

    fn restart_to_start(&mut self) {
        self.stop_worker();
        let summary = self.workflow.summary();
        let session = self.workflow.session().cloned();
        let complete = summary.map(|summary| summary.complete).unwrap_or(false);
        if complete {
            // A completed batch should start clean next time.
            let _ = storage::clear_session();
            self.pending_session = None;
            self.startup_resume_available = false;
        } else if let Some(session) = session {
            // An aborted batch remains resumable.
            self.pending_session = Some(session);
            self.startup_resume_available = true;
        } else {
            self.pending_session = None;
            self.startup_resume_available = false;
        }
        self.workflow = Workflow::new();
        self.dialog_mode = DialogMode::None;
        self.diagnostics = false;
        self.preview_active = false;
        self.preview_step = 0;
        self.removal_started = false;
        self.tag_ndef_memory = 0;
        self.ui
            .show_startup(self.startup_resume_available, self.debug_enabled);
    }

    fn exit(&mut self) {
        self.stop_worker();
        self.ui.stop();
    }
}

fn push_path_text(target: &mut FuriString, path: &[u8]) {
    for &byte in path {
        target.push(byte as char);
    }
}

fn push_hex(target: &mut FuriString, bytes: &[u8]) {
    for byte in bytes {
        let _ = write!(target, "{byte:02X}");
    }
}

pub unsafe extern "C" fn startup_item_callback(context: *mut c_void, index: u32) {
    if context.is_null() {
        return;
    }
    let app = unsafe { &mut *(context as *mut App) };
    app.on_startup_item(index);
}

pub unsafe extern "C" fn folder_item_callback(context: *mut c_void, index: u32) {
    if context.is_null() {
        return;
    }
    let app = unsafe { &mut *(context as *mut App) };
    app.on_folder_item(index);
}

pub unsafe extern "C" fn playlist_item_callback(context: *mut c_void, index: u32) {
    if context.is_null() {
        return;
    }
    let app = unsafe { &mut *(context as *mut App) };
    app.on_playlist_item(index);
}

pub unsafe extern "C" fn dialog_callback(result: sys::DialogExResult, context: *mut c_void) {
    if context.is_null() {
        return;
    }
    let app = unsafe { &mut *(context as *mut App) };
    app.on_dialog(result);
}

pub unsafe extern "C" fn custom_event_callback(context: *mut c_void, event: u32) -> bool {
    if context.is_null() {
        return false;
    }
    let app = unsafe { &mut *(context as *mut App) };
    if event == EVENT_BACK_PENDING {
        let long = app.long_back.swap(false, Ordering::SeqCst);
        storage::trace(if long {
            "back pending long"
        } else {
            "back pending short"
        });
        if long {
            app.show_exit_confirmation();
        } else {
            app.on_back();
        }
        return true;
    }
    if event == EVENT_AUTO_START {
        match app.auto_mode {
            AutoMode::Read => app.start_auto_read(),
            AutoMode::Write => app.start_auto_write(),
            AutoMode::None => {}
        }
        return true;
    }
    if event == EVENT_NFC_TIMEOUT {
        if let Some(worker) = app.worker.as_ref() {
            worker.cancel();
        }
        return true;
    }
    if event != EVENT_NFC_RESULT {
        return false;
    }
    let result = app.worker.as_ref().and_then(|worker| worker.poll());
    if let Some(result) = result {
        app.disarm_operation_timeout();
        app.handle_nfc_result(result);
    }
    true
}

pub unsafe extern "C" fn navigation_callback(context: *mut c_void) -> bool {
    if context.is_null() {
        return false;
    }
    let app = unsafe { &mut *(context as *mut App) };
    if !app.back_timer.is_null() {
        unsafe {
            sys::furi_timer_stop(app.back_timer);
            sys::furi_timer_start(app.back_timer, sys::furi_ms_to_ticks(100));
        }
    } else {
        app.on_back();
    }
    true
}

pub unsafe extern "C" fn back_pending_callback(context: *mut c_void) {
    if context.is_null() {
        return;
    }
    let dispatcher = context as *mut sys::ViewDispatcher;
    unsafe { sys::view_dispatcher_send_custom_event(dispatcher, EVENT_BACK_PENDING) };
}

pub unsafe extern "C" fn input_event_callback(
    message: *const core::ffi::c_void,
    context: *mut c_void,
) {
    if message.is_null() || context.is_null() {
        return;
    }
    let event = unsafe { &*(message as *const sys::InputEvent) };
    if event.key == sys::InputKeyBack && event.type_ == sys::InputTypeLong {
        let app = unsafe { &*(context as *const App) };
        app.long_back.store(true, Ordering::SeqCst);
    }
}

pub unsafe extern "C" fn auto_start_callback(context: *mut c_void) {
    if context.is_null() {
        return;
    }
    let dispatcher = context as *mut sys::ViewDispatcher;
    unsafe { sys::view_dispatcher_send_custom_event(dispatcher, EVENT_AUTO_START) };
}

pub unsafe extern "C" fn operation_timeout_callback(context: *mut c_void) {
    if context.is_null() {
        return;
    }
    // This runs on the timer service thread. Only touch the dispatcher (which
    // is thread-safe); the GUI thread performs the actual cancellation.
    let dispatcher = context as *mut sys::ViewDispatcher;
    unsafe { sys::view_dispatcher_send_custom_event(dispatcher, EVENT_NFC_TIMEOUT) };
}

pub unsafe extern "C" fn preview_popup_callback(context: *mut c_void) {
    if context.is_null() {
        return;
    }
    let app = unsafe { &mut *(context as *mut App) };
    if app.preview_active {
        app.advance_preview();
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}
