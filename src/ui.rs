//! Native Flipper views used by the application.
//!
//! The flow is linear, so a view dispatcher with a Submenu (startup,
//! playlist), a Popup (waiting/writing) and a DialogEx (confirmation, success,
//! error, summary) covers every required screen without a scene manager.

use core::ffi::{CStr, c_void};
use core::fmt::Write as FmtWrite;

use flipperzero::furi::string::FuriString;
use flipperzero::gui::Gui;
use flipperzero_sys as sys;
use nfc_bulk_url_writer_core::name::FixedName;
use nfc_bulk_url_writer_core::playlist::PlaylistStats;
use nfc_bulk_url_writer_core::workflow::{Summary, WorkflowErrorKind};

use crate::storage::{FolderList, PlaylistList};

pub const VIEW_STARTUP: u32 = 0;
pub const VIEW_PLAYLIST: u32 = 1;
pub const VIEW_POPUP: u32 = 2;
pub const VIEW_DIALOG: u32 = 3;
pub const VIEW_FOLDER: u32 = 4;
pub const VIEW_WAITING: u32 = 5;

pub const FOLDER_USE: u32 = u32::MAX;
pub const FOLDER_PARENT: u32 = u32::MAX - 1;

/// Raw view set and dispatcher.
pub struct Ui {
    pub dispatcher: *mut sys::ViewDispatcher,
    pub startup: *mut sys::Submenu,
    pub playlist: *mut sys::Submenu,
    pub popup: *mut sys::Popup,
    pub dialog: *mut sys::DialogEx,
    pub folder: *mut sys::Submenu,
    pub waiting: *mut sys::Widget,
    app_context: *mut c_void,
    writing_header: FuriString,
    writing_text: FuriString,
}

impl Ui {
    pub fn new() -> Self {
        unsafe {
            let dispatcher = sys::view_dispatcher_alloc();
            let startup = sys::submenu_alloc();
            let playlist = sys::submenu_alloc();
            let popup = sys::popup_alloc();
            let dialog = sys::dialog_ex_alloc();
            let folder = sys::submenu_alloc();
            let waiting = sys::widget_alloc();
            sys::view_dispatcher_add_view(dispatcher, VIEW_STARTUP, sys::submenu_get_view(startup));
            sys::view_dispatcher_add_view(
                dispatcher,
                VIEW_PLAYLIST,
                sys::submenu_get_view(playlist),
            );
            sys::view_dispatcher_add_view(dispatcher, VIEW_POPUP, sys::popup_get_view(popup));
            sys::view_dispatcher_add_view(dispatcher, VIEW_DIALOG, sys::dialog_ex_get_view(dialog));
            sys::view_dispatcher_add_view(dispatcher, VIEW_FOLDER, sys::submenu_get_view(folder));
            sys::view_dispatcher_add_view(dispatcher, VIEW_WAITING, sys::widget_get_view(waiting));
            Self {
                dispatcher,
                startup,
                playlist,
                popup,
                dialog,
                folder,
                waiting,
                app_context: core::ptr::null_mut(),
                writing_header: FuriString::new(),
                writing_text: FuriString::new(),
            }
        }
    }

    pub fn set_app_context(&mut self, context: *mut c_void) {
        self.app_context = context;
    }

    pub fn attach(&self, gui: &Gui) {
        unsafe {
            sys::view_dispatcher_attach_to_gui(
                self.dispatcher,
                gui.as_ptr(),
                sys::ViewDispatcherTypeFullscreen,
            );
        }
    }

    pub fn stop(&self) {
        unsafe { sys::view_dispatcher_stop(self.dispatcher) }
    }

    pub fn set_custom_event_callback(&self, callback: sys::ViewDispatcherCustomEventCallback) {
        unsafe { sys::view_dispatcher_set_custom_event_callback(self.dispatcher, callback) }
    }

    pub fn set_navigation_event_callback(
        &self,
        callback: sys::ViewDispatcherNavigationEventCallback,
    ) {
        unsafe { sys::view_dispatcher_set_navigation_event_callback(self.dispatcher, callback) }
    }

    pub fn set_event_context(&self, context: *mut c_void) {
        unsafe { sys::view_dispatcher_set_event_callback_context(self.dispatcher, context) }
    }

    pub fn switch_to(&self, view: u32) {
        unsafe { sys::view_dispatcher_switch_to_view(self.dispatcher, view) }
    }

    fn dialog_setup(&self) {
        unsafe {
            sys::dialog_ex_set_result_callback(self.dialog, Some(crate::app::dialog_callback));
            sys::dialog_ex_set_context(self.dialog, self.app_context);
        }
    }

    pub fn show_startup(&self, can_resume: bool, debug: bool) {
        unsafe {
            sys::submenu_reset(self.startup);
            sys::submenu_set_header(self.startup, c"NFC Bulk URL Writer".as_ptr());
            if can_resume {
                sys::submenu_add_item(
                    self.startup,
                    c"Resume session".as_ptr(),
                    0,
                    Some(crate::app::startup_item_callback),
                    self.app_context,
                );
                sys::submenu_add_item(
                    self.startup,
                    c"Start new session".as_ptr(),
                    1,
                    Some(crate::app::startup_item_callback),
                    self.app_context,
                );
                sys::submenu_add_item(
                    self.startup,
                    c"Load batch from storage".as_ptr(),
                    2,
                    Some(crate::app::startup_item_callback),
                    self.app_context,
                );
            } else {
                sys::submenu_add_item(
                    self.startup,
                    c"Start new session".as_ptr(),
                    0,
                    Some(crate::app::startup_item_callback),
                    self.app_context,
                );
                sys::submenu_add_item(
                    self.startup,
                    c"Load batch from storage".as_ptr(),
                    1,
                    Some(crate::app::startup_item_callback),
                    self.app_context,
                );
            }
            if debug {
                let diagnostics_index = if can_resume { 3 } else { 2 };
                sys::submenu_add_item(
                    self.startup,
                    c"NFC read diagnostics".as_ptr(),
                    diagnostics_index,
                    Some(crate::app::startup_item_callback),
                    self.app_context,
                );
                sys::submenu_add_item(
                    self.startup,
                    c"Preview UI".as_ptr(),
                    diagnostics_index + 1,
                    Some(crate::app::startup_item_callback),
                    self.app_context,
                );
            }
            self.switch_to(VIEW_STARTUP);
        }
    }

    pub fn show_playlists(&self, list: &PlaylistList) {
        unsafe {
            sys::submenu_reset(self.playlist);
            sys::submenu_set_header(self.playlist, c"Select playlist".as_ptr());
            let mut label = FuriString::new();
            for index in 0..list.len() {
                let Some(name) = list.get(index) else {
                    continue;
                };
                label.clear();
                push_bytes(&mut label, name.as_bytes());
                sys::submenu_add_item(
                    self.playlist,
                    label.as_c_ptr(),
                    index as u32,
                    Some(crate::app::playlist_item_callback),
                    self.app_context,
                );
            }
            self.switch_to(VIEW_PLAYLIST);
        }
    }

    pub fn show_folder(&mut self, header: &[u8], folders: &FolderList, has_parent: bool) {
        let mut title = FuriString::new();
        title.push_str("Folder: ");
        push_bytes(&mut title, header);
        unsafe {
            sys::submenu_reset(self.folder);
            sys::submenu_set_header(self.folder, title.as_c_ptr());
            sys::submenu_add_item(
                self.folder,
                c"Use this folder".as_ptr(),
                FOLDER_USE,
                Some(crate::app::folder_item_callback),
                self.app_context,
            );
            if has_parent {
                sys::submenu_add_item(
                    self.folder,
                    c"..".as_ptr(),
                    FOLDER_PARENT,
                    Some(crate::app::folder_item_callback),
                    self.app_context,
                );
            }
            let mut label = FuriString::new();
            for index in 0..folders.len() {
                let Some(name) = folders.get(index) else {
                    continue;
                };
                label.clear();
                push_bytes(&mut label, name.as_bytes());
                sys::submenu_add_item(
                    self.folder,
                    label.as_c_ptr(),
                    index as u32,
                    Some(crate::app::folder_item_callback),
                    self.app_context,
                );
            }
            self.switch_to(VIEW_FOLDER);
        }
    }

    pub fn show_playlist_confirm(&self, name: &FixedName, stats: &PlaylistStats) {
        let mut text = FuriString::new();
        let _ = write!(
            text,
            "{} URLs\n{}",
            stats.total,
            name.as_str().unwrap_or("playlist")
        );
        unsafe {
            sys::dialog_ex_reset(self.dialog);
            sys::dialog_ex_set_header(
                self.dialog,
                c"Start batch?".as_ptr(),
                64,
                2,
                sys::AlignCenter,
                sys::AlignTop,
            );
            sys::dialog_ex_set_text(
                self.dialog,
                text.as_c_ptr(),
                2,
                20,
                sys::AlignLeft,
                sys::AlignTop,
            );
            sys::dialog_ex_set_left_button_text(self.dialog, c"Back".as_ptr());
            sys::dialog_ex_set_center_button_text(self.dialog, c"Start".as_ptr());
            self.dialog_setup();
            self.switch_to(VIEW_DIALOG);
        }
    }

    pub fn show_waiting(&mut self, index: u32, total: u32, url: &[u8]) {
        let mut header = FuriString::new();
        let _ = write!(header, "Tag {index}/{total}");
        let mut url_text = FuriString::new();
        if url.is_empty() {
            url_text.push_str("No URL");
        } else {
            push_bytes(&mut url_text, url);
        }
        let mut state = FuriString::new();
        state.push_str(if index > 1 {
            "Waiting for next tag"
        } else {
            "Waiting for tag"
        });
        unsafe {
            sys::widget_reset(self.waiting);
            sys::widget_add_string_element(
                self.waiting,
                64,
                2,
                sys::AlignCenter,
                sys::AlignTop,
                sys::FontPrimary,
                header.as_c_ptr(),
            );
            sys::widget_add_string_element(
                self.waiting,
                2,
                12,
                sys::AlignLeft,
                sys::AlignTop,
                sys::FontSecondary,
                state.as_c_ptr(),
            );
            sys::widget_add_text_scroll_element(self.waiting, 0, 30, 128, 34, url_text.as_c_ptr());
            sys::widget_add_string_element(
                self.waiting,
                2,
                21,
                sys::AlignLeft,
                sys::AlignTop,
                sys::FontSecondary,
                c"Place tag near Flipper".as_ptr(),
            );
            self.switch_to(VIEW_WAITING);
        }
    }

    pub fn show_writing(
        &mut self,
        index: u32,
        total: u32,
        model: Option<&str>,
        capacity: Option<u16>,
        verifying: bool,
    ) {
        self.writing_header.clear();
        let _ = write!(
            self.writing_header,
            "{} {index}/{total}",
            if verifying { "Checking" } else { "Writing" }
        );
        self.writing_text.clear();
        if let Some(model) = model {
            let _ = write!(self.writing_text, "{model}");
            if let Some(capacity) = capacity {
                let _ = write!(self.writing_text, " {capacity}B");
            }
            self.writing_text.push_str("\n");
        }
        self.writing_text.push_str(if verifying {
            "Verifying..."
        } else {
            "Do not move..."
        });
        unsafe {
            sys::popup_reset(self.popup);
            sys::popup_set_header(
                self.popup,
                self.writing_header.as_c_ptr(),
                64,
                2,
                sys::AlignCenter,
                sys::AlignTop,
            );
            sys::popup_set_text(
                self.popup,
                self.writing_text.as_c_ptr(),
                2,
                22,
                sys::AlignLeft,
                sys::AlignTop,
            );
            self.switch_to(VIEW_POPUP);
        }
    }

    pub fn show_success(&self, index: u32, total: u32, model: &str) {
        let mut text = FuriString::new();
        let _ = write!(text, "{model}\nURL {index} of {total}\nVerified");
        unsafe {
            sys::dialog_ex_reset(self.dialog);
            sys::dialog_ex_set_header(
                self.dialog,
                c"Success".as_ptr(),
                64,
                2,
                sys::AlignCenter,
                sys::AlignTop,
            );
            sys::dialog_ex_set_text(
                self.dialog,
                text.as_c_ptr(),
                2,
                20,
                sys::AlignLeft,
                sys::AlignTop,
            );
            sys::dialog_ex_set_center_button_text(self.dialog, c"OK".as_ptr());
            self.dialog_setup();
            self.switch_to(VIEW_DIALOG);
        }
    }

    pub fn show_remove_tag(&self, same_tag: bool) {
        let text = if same_tag {
            c"Same tag detected.\nRemove it to continue."
        } else {
            c"Remove the programmed\ntag, then place the\nnext one."
        };
        unsafe {
            sys::dialog_ex_reset(self.dialog);
            sys::dialog_ex_set_header(
                self.dialog,
                c"Remove tag".as_ptr(),
                64,
                2,
                sys::AlignCenter,
                sys::AlignTop,
            );
            sys::dialog_ex_set_text(
                self.dialog,
                text.as_ptr(),
                2,
                20,
                sys::AlignLeft,
                sys::AlignTop,
            );
            self.dialog_setup();
            self.switch_to(VIEW_DIALOG);
        }
    }

    pub fn show_error(&self, kind: WorkflowErrorKind) {
        let header = match kind {
            WorkflowErrorKind::Playlist(_) => c"Playlist error",
            WorkflowErrorKind::Session(_) => c"Session error",
            WorkflowErrorKind::Storage => c"Storage error",
            _ => c"Tag error",
        };
        let mut detail = FuriString::new();
        match kind {
            WorkflowErrorKind::Tag(error) => {
                let _ = write!(detail, "{error}");
            }
            WorkflowErrorKind::Ndef(error) => {
                let _ = write!(detail, "{error}");
            }
            WorkflowErrorKind::Playlist(error) => {
                let _ = write!(detail, "{error}");
            }
            WorkflowErrorKind::Session(error) => {
                let _ = write!(detail, "{error}");
            }
            WorkflowErrorKind::WriteFailed => {
                detail.push_str("The write failed.\nTag moved or locked.");
            }
            WorkflowErrorKind::ReadBackFailed => {
                detail.push_str("Could not read the\ntag back.");
            }
            WorkflowErrorKind::TagChanged => {
                detail.push_str("The tag changed or\nwas removed.");
            }
            WorkflowErrorKind::Storage => {
                detail.push_str("SD card access failed.");
            }
        }
        let mut text = FuriString::new();
        push_wrapped(&mut text, detail.as_c_str().to_bytes(), 20, 4);
        unsafe {
            sys::dialog_ex_reset(self.dialog);
            sys::dialog_ex_set_header(
                self.dialog,
                header.as_ptr(),
                64,
                2,
                sys::AlignCenter,
                sys::AlignTop,
            );
            sys::dialog_ex_set_text(
                self.dialog,
                text.as_c_ptr(),
                2,
                20,
                sys::AlignLeft,
                sys::AlignTop,
            );
            if kind.is_tag_error() {
                sys::dialog_ex_set_left_button_text(self.dialog, c"Retry".as_ptr());
                sys::dialog_ex_set_center_button_text(self.dialog, c"Skip".as_ptr());
                sys::dialog_ex_set_right_button_text(self.dialog, c"Abort".as_ptr());
            } else {
                sys::dialog_ex_set_left_button_text(self.dialog, c"New".as_ptr());
                sys::dialog_ex_set_center_button_text(self.dialog, c"Exit".as_ptr());
            }
            self.dialog_setup();
            self.switch_to(VIEW_DIALOG);
        }
    }

    pub fn show_confirmation(&self, header: &CStr, message: &CStr, left: &CStr, center: &CStr) {
        unsafe {
            sys::dialog_ex_reset(self.dialog);
            sys::dialog_ex_set_header(
                self.dialog,
                header.as_ptr(),
                64,
                2,
                sys::AlignCenter,
                sys::AlignTop,
            );
            sys::dialog_ex_set_text(
                self.dialog,
                message.as_ptr(),
                2,
                20,
                sys::AlignLeft,
                sys::AlignTop,
            );
            sys::dialog_ex_set_left_button_text(self.dialog, left.as_ptr());
            sys::dialog_ex_set_center_button_text(self.dialog, center.as_ptr());
            self.dialog_setup();
            self.switch_to(VIEW_DIALOG);
        }
    }

    pub fn enable_popup_timeout(&self, milliseconds: u32) {
        unsafe {
            sys::popup_set_timeout(self.popup, milliseconds);
            sys::popup_enable_timeout(self.popup);
        }
    }

    pub fn popup_set_timeout_callback(&self, callback: sys::PopupCallback) {
        unsafe {
            sys::popup_set_callback(self.popup, callback);
            sys::popup_set_context(self.popup, self.app_context);
        }
    }

    pub fn show_summary(&self, summary: Summary, allow_new: bool) {
        let mut header = FuriString::new();
        header.push_str(if summary.complete {
            "Batch complete"
        } else {
            "Session aborted"
        });
        let mut text = FuriString::new();
        let _ = write!(
            text,
            "Done {}/{}\nSkipped {}\nFailed {}\nRemaining {}",
            summary.completed, summary.total, summary.skipped, summary.failed, summary.remaining
        );
        unsafe {
            sys::dialog_ex_reset(self.dialog);
            sys::dialog_ex_set_header(
                self.dialog,
                header.as_c_ptr(),
                64,
                2,
                sys::AlignCenter,
                sys::AlignTop,
            );
            sys::dialog_ex_set_text(
                self.dialog,
                text.as_c_ptr(),
                2,
                18,
                sys::AlignLeft,
                sys::AlignTop,
            );
            if allow_new {
                sys::dialog_ex_set_left_button_text(self.dialog, c"New".as_ptr());
            }
            sys::dialog_ex_set_center_button_text(self.dialog, c"Done".as_ptr());
            self.dialog_setup();
            self.switch_to(VIEW_DIALOG);
        }
    }

    pub fn show_message_dynamic(&self, header: &CStr, message: &FuriString, center: &CStr) {
        unsafe {
            sys::dialog_ex_reset(self.dialog);
            sys::dialog_ex_set_header(
                self.dialog,
                header.as_ptr(),
                64,
                2,
                sys::AlignCenter,
                sys::AlignTop,
            );
            sys::dialog_ex_set_text(
                self.dialog,
                message.as_c_ptr(),
                2,
                20,
                sys::AlignLeft,
                sys::AlignTop,
            );
            sys::dialog_ex_set_center_button_text(self.dialog, center.as_ptr());
            self.dialog_setup();
            self.switch_to(VIEW_DIALOG);
        }
    }

    pub fn show_message(&self, header: &CStr, message: &CStr, center: &CStr) {
        unsafe {
            sys::dialog_ex_reset(self.dialog);
            sys::dialog_ex_set_header(
                self.dialog,
                header.as_ptr(),
                64,
                2,
                sys::AlignCenter,
                sys::AlignTop,
            );
            sys::dialog_ex_set_text(
                self.dialog,
                message.as_ptr(),
                2,
                20,
                sys::AlignLeft,
                sys::AlignTop,
            );
            sys::dialog_ex_set_center_button_text(self.dialog, center.as_ptr());
            self.dialog_setup();
            self.switch_to(VIEW_DIALOG);
        }
    }

    pub fn free(&mut self) {
        unsafe {
            sys::view_dispatcher_remove_view(self.dispatcher, VIEW_STARTUP);
            sys::view_dispatcher_remove_view(self.dispatcher, VIEW_PLAYLIST);
            sys::view_dispatcher_remove_view(self.dispatcher, VIEW_POPUP);
            sys::view_dispatcher_remove_view(self.dispatcher, VIEW_DIALOG);
            sys::view_dispatcher_remove_view(self.dispatcher, VIEW_FOLDER);
            sys::view_dispatcher_remove_view(self.dispatcher, VIEW_WAITING);
            sys::submenu_free(self.startup);
            sys::submenu_free(self.playlist);
            sys::submenu_free(self.folder);
            sys::widget_free(self.waiting);
            sys::popup_free(self.popup);
            sys::dialog_ex_free(self.dialog);
            sys::view_dispatcher_free(self.dispatcher);
        }
        self.startup = core::ptr::null_mut();
        self.playlist = core::ptr::null_mut();
        self.popup = core::ptr::null_mut();
        self.dialog = core::ptr::null_mut();
        self.folder = core::ptr::null_mut();
        self.waiting = core::ptr::null_mut();
        self.dispatcher = core::ptr::null_mut();
    }
}

fn push_bytes(target: &mut FuriString, bytes: &[u8]) {
    for &byte in bytes {
        target.push(byte as char);
    }
}

fn push_wrapped(target: &mut FuriString, bytes: &[u8], width: usize, max_lines: usize) {
    let mut line_len = 0usize;
    let mut lines = 1usize;
    let mut truncated = false;
    for &byte in bytes {
        if byte == b'\n' {
            if lines == max_lines {
                truncated = true;
                break;
            }
            target.push('\n');
            line_len = 0;
            lines += 1;
            continue;
        }
        if line_len == width {
            if lines == max_lines {
                truncated = true;
                break;
            }
            target.push('\n');
            line_len = 0;
            lines += 1;
        }
        target.push(if byte.is_ascii_graphic() || byte == b' ' {
            byte as char
        } else {
            ' '
        });
        line_len += 1;
    }
    if truncated {
        target.push_str("...");
    }
}
