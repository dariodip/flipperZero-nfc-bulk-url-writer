//! Dedicated NFC worker thread.
//!
//! The worker owns the `Nfc` handle and every poller. It uses a custom
//! `nfc_poller_start_ex` session and the exported MfUltralight per-page
//! functions. This avoids the high-level read state machine (which probes the
//! default password) and the firmware write mode (which requires an
//! authenticated session and rewrites the whole user area).

use core::ffi::c_void;
use core::marker::PhantomData;
use core::mem::size_of;
use core::ptr;
use core::sync::atomic::{AtomicBool, Ordering};

use flipperzero_sys as sys;
use nfc_bulk_url_writer_core::errors::{NdefError, TagError};
use nfc_bulk_url_writer_core::ndef;
use nfc_bulk_url_writer_core::tag::{self, MAX_UID_LEN, TagModel, TagSnapshot, VERSION_LEN};
use nfc_bulk_url_writer_core::{MAX_TLV_LEN, MAX_URL_LEN};

use super::compat;

/// Custom event sent to the view dispatcher when a worker result is available.
pub const EVENT_NFC_RESULT: u32 = 0x100;

const NFC_COMMAND_CONTINUE: u32 = 0;
const NFC_COMMAND_STOP: u32 = 2;
const COMPLETION_FLAG: u32 = 1;

/// Commands accepted by the NFC worker.
#[allow(clippy::large_enum_variant)]
pub enum NfcRequest {
    /// Read the tag metadata (and optionally a full user-area dump).
    ReadAndValidate { required_tlv: u16, full_dump: bool },
    /// Re-read the tag, confirm its UID, write the TLV, read back and verify.
    ///
    /// `model` carries the model that was validated during the immediately
    /// preceding read phase; when known, the write session skips the
    /// GET_VERSION exchange and reconstructs the model identity from it.
    WriteAndVerify {
        required_tlv: u16,
        url_len: u16,
        url: [u8; MAX_URL_LEN],
        model: Option<TagModel>,
    },
    /// Wait until the programmed tag is removed or replaced.
    CheckRemoval { uid: [u8; MAX_UID_LEN], uid_len: u8 },
    /// Stop the worker thread.
    Stop,
}

/// Results posted by the worker.
#[derive(Clone, Copy)]
pub enum NfcResult {
    Validated {
        model: TagModel,
        ndef_memory: u16,
        uid: [u8; MAX_UID_LEN],
        uid_len: u8,
        version: [u8; 8],
        pages_read: u16,
        cc: [u8; 4],
        static_lock: [u8; 2],
        dynamic_lock: [u8; 4],
        config: [u8; 8],
    },
    NdefRejected(NdefError),
    Rejected(TagError),
    TagChanged,
    WriteFailed,
    ReadbackFailed,
    VerificationFailed,
    Verified,
    SameTagPresent,
    TagCleared,
    InternalError,
}

/// Small wrapper around a raw Furi message queue.
struct RawQueue<T> {
    handle: *mut sys::FuriMessageQueue,
    _marker: PhantomData<T>,
}

impl<T> RawQueue<T> {
    fn new(capacity: u32) -> Option<Self> {
        let handle = unsafe { sys::furi_message_queue_alloc(capacity, size_of::<T>() as u32) };
        if handle.is_null() {
            None
        } else {
            Some(Self {
                handle,
                _marker: PhantomData,
            })
        }
    }

    fn handle(&self) -> *mut sys::FuriMessageQueue {
        self.handle
    }
}

impl<T> Drop for RawQueue<T> {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { sys::furi_message_queue_free(self.handle) }
        }
    }
}

fn queue_put<T>(handle: *mut sys::FuriMessageQueue, message: &T, timeout: u32) -> bool {
    unsafe {
        sys::furi_message_queue_put(handle, message as *const T as *const c_void, timeout)
            == sys::FuriStatusOk
    }
}

fn queue_get<T>(handle: *mut sys::FuriMessageQueue, out: &mut T, timeout: u32) -> bool {
    unsafe {
        sys::furi_message_queue_get(handle, out as *mut T as *mut c_void, timeout)
            == sys::FuriStatusOk
    }
}

struct WorkerContext {
    commands: *mut sys::FuriMessageQueue,
    events: *mut sys::FuriMessageQueue,
    dispatcher: *mut sys::ViewDispatcher,
    nfc: *mut sys::Nfc,
    cancel: AtomicBool,
    last_uid: [u8; MAX_UID_LEN],
    last_uid_len: u8,
}

impl WorkerContext {
    fn emit(&self, result: NfcResult) {
        let _ = queue_put(self.events, &result, 10);
        unsafe { sys::view_dispatcher_send_custom_event(self.dispatcher, EVENT_NFC_RESULT) };
    }
}

/// NFC worker handle owned by the application.
pub struct NfcWorker {
    commands: RawQueue<NfcRequest>,
    events: RawQueue<NfcResult>,
    thread: *mut sys::FuriThread,
    context: *mut WorkerContext,
}

impl NfcWorker {
    /// Starts the worker thread with an `Nfc` handle allocated by the caller.
    pub fn start(dispatcher: *mut sys::ViewDispatcher, nfc: *mut sys::Nfc) -> Option<Self> {
        if nfc.is_null() {
            return None;
        }
        let commands = RawQueue::<NfcRequest>::new(2)?;
        let events = RawQueue::<NfcResult>::new(4)?;
        let allocation =
            unsafe { sys::malloc(size_of::<WorkerContext>() as u32) as *mut WorkerContext };
        if allocation.is_null() {
            unsafe { sys::nfc_free(nfc) };
            return None;
        }
        unsafe {
            ptr::write(
                allocation,
                WorkerContext {
                    commands: commands.handle(),
                    events: events.handle(),
                    dispatcher,
                    nfc,
                    cancel: AtomicBool::new(false),
                    last_uid: [0u8; MAX_UID_LEN],
                    last_uid_len: 0,
                },
            );
        }

        let thread = unsafe {
            sys::furi_thread_alloc_ex(
                c"NfcBulkUrlWr".as_ptr(),
                12288,
                Some(worker_entry),
                allocation as *mut c_void,
            )
        };
        if thread.is_null() {
            unsafe {
                ptr::drop_in_place(allocation);
                sys::free(allocation as *mut c_void);
                sys::nfc_free(nfc);
            }
            return None;
        }
        unsafe { sys::furi_thread_start(thread) };

        Some(Self {
            commands,
            events,
            thread,
            context: allocation,
        })
    }

    /// Sends a command to the worker.
    pub fn send(&self, request: NfcRequest) -> bool {
        queue_put(self.commands.handle(), &request, 100)
    }

    /// Polls for one worker result.
    pub fn poll(&self) -> Option<NfcResult> {
        let mut result = NfcResult::InternalError;
        if queue_get(self.events.handle(), &mut result, 0) {
            Some(result)
        } else {
            None
        }
    }

    /// Asks the worker to stop a blocking poller operation.
    pub fn cancel(&self) {
        if self.context.is_null() {
            return;
        }
        unsafe {
            let context = &*self.context;
            context.cancel.store(true, Ordering::SeqCst);
        }
    }

    /// Stops and joins the worker, freeing all resources.
    pub fn stop(&mut self) {
        if self.thread.is_null() {
            return;
        }
        self.cancel();
        let _ = queue_put(self.commands.handle(), &NfcRequest::Stop, 100);
        unsafe {
            sys::furi_thread_join(self.thread);
            sys::furi_thread_free(self.thread);
        }
        self.thread = ptr::null_mut();

        if !self.context.is_null() {
            unsafe {
                let context = &mut *self.context;
                if !context.nfc.is_null() {
                    sys::nfc_free(context.nfc);
                    context.nfc = ptr::null_mut();
                }
                ptr::drop_in_place(self.context);
                sys::free(self.context as *mut c_void);
            }
            self.context = ptr::null_mut();
        }
    }
}

impl Drop for NfcWorker {
    fn drop(&mut self) {
        self.stop();
    }
}

unsafe extern "C" fn worker_entry(context: *mut c_void) -> i32 {
    if context.is_null() {
        return 1;
    }
    let context = unsafe { &mut *(context as *mut WorkerContext) };
    worker_loop(context);
    0
}

fn worker_loop(context: &mut WorkerContext) {
    loop {
        let mut request = NfcRequest::Stop;
        if !queue_get(context.commands, &mut request, 100) {
            continue;
        }
        match request {
            NfcRequest::Stop => break,
            NfcRequest::ReadAndValidate {
                required_tlv,
                full_dump,
            } => {
                if !handle_read(context, required_tlv as usize, full_dump) {
                    break;
                }
            }
            NfcRequest::WriteAndVerify {
                required_tlv,
                url_len,
                url,
                model,
            } => {
                let length = (url_len as usize).min(MAX_URL_LEN);
                if !handle_write(context, required_tlv as usize, &url[..length], model) {
                    break;
                }
            }
            NfcRequest::CheckRemoval { uid, uid_len } => {
                if !handle_removal(context, uid, uid_len) {
                    break;
                }
            }
        }
    }
}

enum WaitOutcome {
    Tag,
    NoTag,
    Stop,
}

fn detect_once(context: &mut WorkerContext) -> WaitOutcome {
    let mut request = NfcRequest::Stop;
    if queue_get(context.commands, &mut request, 0) && matches!(request, NfcRequest::Stop) {
        return WaitOutcome::Stop;
    }

    if context.cancel.load(Ordering::SeqCst) {
        context.cancel.store(false, Ordering::SeqCst);
        return WaitOutcome::Stop;
    }
    let poller = unsafe { sys::nfc_poller_alloc(context.nfc, sys::NfcProtocolMfUltralight) };
    if poller.is_null() {
        return WaitOutcome::NoTag;
    }
    let detected = unsafe { sys::nfc_poller_detect(poller) };
    unsafe { sys::nfc_poller_free(poller) };

    if detected {
        WaitOutcome::Tag
    } else {
        WaitOutcome::NoTag
    }
}

/// Streaming the model into `Operation::Write` lets the write session skip
/// the GET_VERSION round-trip when the model was validated moments before.
#[derive(Clone, Copy)]
enum Operation<'a> {
    Read {
        required_tlv: usize,
        full_dump: bool,
    },
    Write {
        tlv: &'a [u8],
        url: &'a [u8],
        expected_uid: [u8; MAX_UID_LEN],
        expected_uid_len: u8,
        model: Option<TagModel>,
    },
    Removal {
        expected_uid: [u8; MAX_UID_LEN],
        expected_uid_len: u8,
    },
}

struct SessionCallback<'a> {
    operation: Operation<'a>,
    snapshot: TagSnapshot,
    result: Option<NfcResult>,
    thread_id: *mut c_void,
}

impl SessionCallback<'_> {
    fn complete(&self) {
        if !self.thread_id.is_null() {
            unsafe { sys::furi_thread_flags_set(self.thread_id, COMPLETION_FLAG) };
        }
    }
}

// The firmware's NfcCommand is a 4-byte C enum, while the generated binding
// represents it as a 1-byte struct. Returning the binding type would leave the
// upper 24 bits of r0 unspecified, so use an explicit 32-bit return.
unsafe extern "C" fn session_callback(event: sys::NfcGenericEventEx, context: *mut c_void) -> u32 {
    let callback = unsafe { &mut *(context as *mut SessionCallback) };
    if event.poller.is_null() || event.parent_event_data.is_null() {
        callback.result = Some(NfcResult::InternalError);
        callback.complete();
        return NFC_COMMAND_STOP;
    }

    let parent = unsafe { &*(event.parent_event_data as *const sys::Iso14443_3aPollerEvent) };
    if parent.type_ != sys::Iso14443_3aPollerEventTypeReady {
        // No tag yet, or a transient activation failure. Keep the session
        // alive and let the parent poller try again; the UI is still on the
        // "waiting for tag" screen.
        return NFC_COMMAND_CONTINUE;
    }

    let poller = event.poller as *mut sys::MfUltralightPoller;
    callback.result = Some(callback.execute(poller));
    callback.complete();
    NFC_COMMAND_STOP
}

impl SessionCallback<'_> {
    fn execute(&mut self, poller: *mut sys::MfUltralightPoller) -> NfcResult {
        match self.operation {
            Operation::Read {
                required_tlv,
                full_dump,
            } => self.execute_read(poller, required_tlv, full_dump),
            Operation::Write {
                tlv,
                url,
                expected_uid,
                expected_uid_len,
                model,
            } => self.execute_write(poller, tlv, url, expected_uid, expected_uid_len, model),
            Operation::Removal {
                expected_uid,
                expected_uid_len,
            } => self.execute_removal(poller, expected_uid, expected_uid_len),
        }
    }

    fn execute_read(
        &mut self,
        poller: *mut sys::MfUltralightPoller,
        required_tlv: usize,
        full_dump: bool,
    ) -> NfcResult {
        let (model, version) = match read_metadata(poller, &mut self.snapshot) {
            Ok(values) => values,
            Err(error) => return NfcResult::Rejected(error),
        };
        if full_dump {
            let last = model.last_user_page();
            if read_pages(poller, &mut self.snapshot, 4, last).is_ok() {
                self.snapshot.set_pages_read(last + 1);
            }
        }

        match tag::validate_metadata(&self.snapshot, required_tlv) {
            Ok(info) => {
                let uid_len = self.snapshot.uid_len;
                let mut uid = [0u8; MAX_UID_LEN];
                uid[..self.snapshot.uid().len()].copy_from_slice(self.snapshot.uid());
                let cc = self.snapshot.page(3).unwrap_or([0u8; 4]);
                let static_page = self.snapshot.page(2).unwrap_or([0u8; 4]);
                let dynamic_lock = self
                    .snapshot
                    .page(info.model.dynamic_lock_page())
                    .unwrap_or([0u8; 4]);
                let config_page = info.model.config_page();
                let config0 = self.snapshot.page(config_page).unwrap_or([0u8; 4]);
                let config1 = self.snapshot.page(config_page + 1).unwrap_or([0u8; 4]);
                let mut config = [0u8; 8];
                config[..4].copy_from_slice(&config0);
                config[4..].copy_from_slice(&config1);
                NfcResult::Validated {
                    model: info.model,
                    ndef_memory: info.ndef_memory as u16,
                    uid,
                    uid_len,
                    version: compat::version_to_bytes(&version),
                    pages_read: self.snapshot.pages_read,
                    cc,
                    static_lock: [static_page[2], static_page[3]],
                    dynamic_lock,
                    config,
                }
            }
            Err(error) => NfcResult::Rejected(error),
        }
    }

    fn execute_write(
        &mut self,
        poller: *mut sys::MfUltralightPoller,
        tlv: &[u8],
        url: &[u8],
        expected_uid: [u8; MAX_UID_LEN],
        expected_uid_len: u8,
        model: Option<TagModel>,
    ) -> NfcResult {
        match model {
            Some(model) => {
                if let Err(error) = read_identity(poller, &mut self.snapshot, model) {
                    return NfcResult::Rejected(error);
                }
            }
            None => {
                if let Err(error) = read_metadata(poller, &mut self.snapshot).map(|_| ()) {
                    return NfcResult::Rejected(error);
                }
            }
        }
        if !same_uid(&self.snapshot, expected_uid, expected_uid_len) {
            return NfcResult::TagChanged;
        }
        let info = match tag::validate_metadata(&self.snapshot, tlv.len()) {
            Ok(info) => info,
            Err(error) => return NfcResult::Rejected(error),
        };
        let model = info.model;

        // Fill the tag's NDEF area with the new TLV followed by zeros. This
        // removes stale bytes from previous messages (some readers mishandle
        // them) while never touching metadata or user memory outside the
        // declared NDEF area.
        let write_end = model.ndef_memory();
        let last_page = tag::TagModel::first_user_page() + (write_end / tag::PAGE_SIZE) as u16 - 1;
        if last_page > model.last_user_page() {
            return NfcResult::Rejected(TagError::InsufficientCapacity {
                required: tlv.len(),
                available: model.ndef_memory(),
            });
        }

        let page_count = write_end / tag::PAGE_SIZE;
        for index in 0..page_count {
            let page_number = tag::TagModel::first_user_page() + index as u16;
            let page_start = index * tag::PAGE_SIZE;
            let mut bytes = [0u8; tag::PAGE_SIZE];
            for (offset, byte) in bytes.iter_mut().enumerate() {
                let global = page_start + offset;
                if global < tlv.len() {
                    *byte = tlv[global];
                }
            }
            let page = sys::MfUltralightPage { data: bytes };
            let error =
                unsafe { sys::mf_ultralight_poller_write_page(poller, page_number as u8, &page) };
            if error != sys::MfUltralightErrorNone {
                return NfcResult::WriteFailed;
            }
        }

        unsafe { sys::furi_delay_ms(5) };

        let mut readback = [0u8; MAX_TLV_LEN];
        let chunks = tlv.len().div_ceil(16);
        for chunk in 0..chunks {
            let start_page = tag::TagModel::first_user_page() + (chunk * 4) as u16;
            let mut command: sys::MfUltralightPageReadCommandData = unsafe { core::mem::zeroed() };
            let error = unsafe {
                sys::mf_ultralight_poller_read_page(poller, start_page as u8, &mut command)
            };
            if error != sys::MfUltralightErrorNone {
                return NfcResult::ReadbackFailed;
            }
            for page in 0..4 {
                let offset = chunk * 16 + page * 4;
                if offset < tlv.len() {
                    let available = (tlv.len() - offset).min(4);
                    readback[offset..offset + available]
                        .copy_from_slice(&command.page[page].data[..available]);
                }
            }
        }

        let mut scratch = [0u8; MAX_URL_LEN];
        match ndef::verify(&readback[..tlv.len()], url, &mut scratch) {
            Ok(_) => NfcResult::Verified,
            Err(NdefError::VerificationMismatch) => NfcResult::VerificationFailed,
            Err(_) => NfcResult::VerificationFailed,
        }
    }

    fn execute_removal(
        &mut self,
        poller: *mut sys::MfUltralightPoller,
        expected_uid: [u8; MAX_UID_LEN],
        expected_uid_len: u8,
    ) -> NfcResult {
        if read_uid(poller, &mut self.snapshot).is_err() {
            // A present but unreadable tag is treated as the programmed one.
            return NfcResult::SameTagPresent;
        }
        if same_uid(&self.snapshot, expected_uid, expected_uid_len) {
            NfcResult::SameTagPresent
        } else {
            NfcResult::TagCleared
        }
    }
}

fn same_uid(snapshot: &TagSnapshot, expected: [u8; MAX_UID_LEN], expected_len: u8) -> bool {
    snapshot.uid_len == expected_len && snapshot.uid() == &expected[..expected_len as usize]
}

fn read_version(
    poller: *mut sys::MfUltralightPoller,
) -> Result<sys::MfUltralightVersion, TagError> {
    let mut version: sys::MfUltralightVersion = unsafe { core::mem::zeroed() };
    let error = unsafe { sys::mf_ultralight_poller_read_version(poller, &mut version) };
    if error == sys::MfUltralightErrorNone {
        Ok(version)
    } else {
        Err(TagError::UnsupportedTag)
    }
}

fn read_pages(
    poller: *mut sys::MfUltralightPoller,
    snapshot: &mut TagSnapshot,
    first: u16,
    last: u16,
) -> Result<(), TagError> {
    let mut start = first;
    while start <= last {
        let mut command: sys::MfUltralightPageReadCommandData = unsafe { core::mem::zeroed() };
        let error =
            unsafe { sys::mf_ultralight_poller_read_page(poller, start as u8, &mut command) };
        if error != sys::MfUltralightErrorNone {
            return Err(TagError::IncompleteDump);
        }
        for offset in 0..4u16 {
            let page = start + offset;
            if page <= last {
                snapshot.set_page(page, command.page[offset as usize].data);
            }
        }
        start += 4;
    }
    Ok(())
}

fn read_metadata(
    poller: *mut sys::MfUltralightPoller,
    snapshot: &mut TagSnapshot,
) -> Result<(TagModel, sys::MfUltralightVersion), TagError> {
    let version = read_version(poller)?;
    let model =
        TagModel::from_storage_size(version.storage_size).ok_or(TagError::UnsupportedTag)?;
    snapshot.set_version(compat::version_to_bytes(&version));

    // UID, static locks, Capability Container.
    read_pages(poller, snapshot, 0, 3)?;
    // Dynamic lock page and configuration pages. The dynamic lock page is
    // immediately before the first configuration page.
    let config = model.config_page();
    read_pages(poller, snapshot, config - 1, config + 2)?;
    read_pages(poller, snapshot, config, config + 3)?;
    snapshot.set_pages_read(12);

    let page0 = snapshot.page(0).unwrap_or([0u8; 4]);
    let page1 = snapshot.page(1).unwrap_or([0u8; 4]);
    // NTAG21x use a 7-byte UID: page 0 holds UID0-2 and BCC0, page 1 holds
    // UID3-6.
    snapshot.set_uid(&[
        page0[0], page0[1], page0[2], page1[0], page1[1], page1[2], page1[3],
    ]);

    Ok((model, version))
}

fn read_uid(
    poller: *mut sys::MfUltralightPoller,
    snapshot: &mut TagSnapshot,
) -> Result<(), TagError> {
    read_pages(poller, snapshot, 0, 1)?;
    let page0 = snapshot.page(0).unwrap_or([0u8; 4]);
    let page1 = snapshot.page(1).unwrap_or([0u8; 4]);
    // NTAG21x use a 7-byte UID: page 0 holds UID0-2 and BCC0, page 1 holds
    // UID3-6.
    snapshot.set_uid(&[
        page0[0], page0[1], page0[2], page1[0], page1[1], page1[2], page1[3],
    ]);
    snapshot.set_pages_read(4);
    Ok(())
}

/// Reads the identity, lock and configuration pages for a model known in
/// advance, skipping the GET_VERSION exchange.
///
/// The write session runs directly after the read session validated the tag,
/// so the model's storage-size byte is reproduced instead of re-read; the
/// page reads themselves are unchanged, keeping the full re-validation of the
/// Capability Container, locks and protection.
fn read_identity(
    poller: *mut sys::MfUltralightPoller,
    snapshot: &mut TagSnapshot,
    model: TagModel,
) -> Result<(), TagError> {
    // UID, static locks, Capability Container.
    read_pages(poller, snapshot, 0, 3)?;
    // Dynamic lock page and configuration pages. The dynamic lock page is
    // immediately before the first configuration page.
    let config = model.config_page();
    read_pages(poller, snapshot, config - 1, config + 2)?;
    read_pages(poller, snapshot, config, config + 3)?;
    snapshot.set_pages_read(12);

    let mut version = [0u8; VERSION_LEN];
    version[6] = model.storage_size();
    snapshot.set_version(version);

    let page0 = snapshot.page(0).unwrap_or([0u8; 4]);
    let page1 = snapshot.page(1).unwrap_or([0u8; 4]);
    // NTAG21x use a 7-byte UID: page 0 holds UID0-2 and BCC0, page 1 holds
    // UID3-6.
    snapshot.set_uid(&[
        page0[0], page0[1], page0[2], page1[0], page1[1], page1[2], page1[3],
    ]);
    Ok(())
}

fn run_session(
    context: &mut WorkerContext,
    operation: Operation<'_>,
    wait: bool,
) -> Option<NfcResult> {
    context.cancel.store(false, Ordering::SeqCst);
    if !wait {
        match detect_once(context) {
            WaitOutcome::Tag => unsafe { sys::furi_delay_ms(50) },
            WaitOutcome::NoTag => return Some(NfcResult::TagCleared),
            WaitOutcome::Stop => return None,
        }
    }

    let poller = unsafe { sys::nfc_poller_alloc(context.nfc, sys::NfcProtocolMfUltralight) };
    if poller.is_null() {
        return Some(NfcResult::InternalError);
    }

    let thread_id = unsafe { sys::furi_thread_get_current_id() };
    unsafe { sys::furi_thread_flags_clear(COMPLETION_FLAG) };

    let mut callback = SessionCallback {
        operation,
        snapshot: TagSnapshot::new(),
        result: None,
        thread_id,
    };
    type Callback32 = unsafe extern "C" fn(sys::NfcGenericEventEx, *mut c_void) -> u32;
    let callback_fn: Callback32 = session_callback;
    let callback_ptr: sys::NfcGenericCallbackEx =
        unsafe { core::mem::transmute(Some(callback_fn)) };
    unsafe {
        sys::nfc_poller_start_ex(
            poller,
            callback_ptr,
            &mut callback as *mut SessionCallback as *mut c_void,
        );
    }

    // nfc_poller_start_ex is asynchronous. Wait for the callback to signal
    // completion, checking for a cancel request between waits.
    loop {
        let flags = unsafe {
            sys::furi_thread_flags_wait(
                COMPLETION_FLAG,
                sys::FuriFlagWaitAny.0,
                sys::furi_ms_to_ticks(100),
            )
        };
        if flags & COMPLETION_FLAG != 0 {
            unsafe { sys::furi_thread_flags_clear(COMPLETION_FLAG) };
            break;
        }
        if context.cancel.swap(false, Ordering::SeqCst) {
            unsafe { sys::nfc_poller_stop(poller) };
            unsafe { sys::nfc_poller_free(poller) };
            return None;
        }
    }

    unsafe { sys::nfc_poller_stop(poller) };
    unsafe { sys::nfc_poller_free(poller) };

    callback.result.or(Some(NfcResult::InternalError))
}

fn handle_read(context: &mut WorkerContext, required_tlv: usize, full_dump: bool) -> bool {
    // The required TLV size is checked against the tag by the core validation.
    if required_tlv > MAX_TLV_LEN {
        context.emit(NfcResult::Rejected(TagError::InsufficientCapacity {
            required: required_tlv,
            available: 0,
        }));
        return true;
    }
    match run_session(
        context,
        Operation::Read {
            required_tlv,
            full_dump,
        },
        true,
    ) {
        Some(result) => {
            if let NfcResult::Validated { uid, uid_len, .. } = result {
                context.last_uid = uid;
                context.last_uid_len = uid_len;
            }
            context.emit(result);
            true
        }
        None => false,
    }
}

fn handle_write(
    context: &mut WorkerContext,
    required_tlv: usize,
    url: &[u8],
    model: Option<TagModel>,
) -> bool {
    let mut tlv = [0u8; MAX_TLV_LEN];
    let encoded = match ndef::encode(url, &mut tlv) {
        Ok(encoded) => encoded,
        Err(error) => {
            context.emit(NfcResult::NdefRejected(error));
            return true;
        }
    };
    if encoded.tlv_len != required_tlv {
        context.emit(NfcResult::NdefRejected(NdefError::BufferTooSmall));
        return true;
    }

    let uid = context.last_uid;
    let uid_len = context.last_uid_len;
    match run_session(
        context,
        Operation::Write {
            tlv: &tlv[..encoded.tlv_len],
            url,
            expected_uid: uid,
            expected_uid_len: uid_len,
            model,
        },
        true,
    ) {
        Some(result) => {
            context.emit(result);
            true
        }
        None => false,
    }
}

fn handle_removal(
    context: &mut WorkerContext,
    expected_uid: [u8; MAX_UID_LEN],
    expected_uid_len: u8,
) -> bool {
    let mut same_tag_reported = false;
    loop {
        let mut request = NfcRequest::Stop;
        if queue_get(context.commands, &mut request, 0) && matches!(request, NfcRequest::Stop) {
            return false;
        }

        match run_session(
            context,
            Operation::Removal {
                expected_uid,
                expected_uid_len,
            },
            false,
        ) {
            Some(NfcResult::TagCleared) => {
                context.emit(NfcResult::TagCleared);
                return true;
            }
            Some(NfcResult::SameTagPresent) => {
                if !same_tag_reported {
                    context.emit(NfcResult::SameTagPresent);
                    same_tag_reported = true;
                }
            }
            Some(_) => {
                // A failed presence read is treated like a present tag.
                if !same_tag_reported {
                    context.emit(NfcResult::SameTagPresent);
                    same_tag_reported = true;
                }
            }
            None => return false,
        }
        unsafe { sys::furi_delay_ms(200) };
    }
}
