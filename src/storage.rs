//! microSD storage adapter: playlist discovery, streaming reads and session I/O.

use core::ffi::{CStr, c_char};

use flipperzero::furi::string::FuriString;
use flipperzero::io::{Error as IoError, Read, Write};
use flipperzero::storage::{File, OpenOptions, Storage};
use flipperzero_sys as sys;
use nfc_bulk_url_writer_core::errors::PlaylistError;
use nfc_bulk_url_writer_core::name::{self, FixedName, MAX_NAME_LEN};
use nfc_bulk_url_writer_core::playlist::{self, LineReadError, LineSource, PlaylistStats};
use nfc_bulk_url_writer_core::session::{MAX_SESSION_LEN, Session};

/// Directory containing user playlists.
pub const PLAYLIST_DIR: &CStr = c"/ext/apps_data/nfc-bulk-url-writer/playlists";
/// User settings file.
pub const SETTINGS_PATH: &CStr = c"/ext/apps_data/nfc-bulk-url-writer/settings.txt";
/// Primary session file.
pub const SESSION_PATH: &CStr = c"/ext/apps_data/nfc-bulk-url-writer/session.txt";
/// Temporary session file used for atomic replacement.
pub const SESSION_TMP_PATH: &CStr = c"/ext/apps_data/nfc-bulk-url-writer/session.tmp";
/// Previous valid session file.
pub const SESSION_BAK_PATH: &CStr = c"/ext/apps_data/nfc-bulk-url-writer/session.bak";

const MAX_PLAYLISTS: usize = 64;
const MAX_FOLDERS: usize = 32;
const DIRECTORY_NAME_LEN: usize = MAX_NAME_LEN + 8;

/// Storage failure categories the UI can distinguish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageError {
    MissingDirectory,
    MissingFile,
    Read,
    Write,
    Corrupt,
}

/// Fixed-capacity list of playlists, sorted by display name.
pub struct PlaylistList {
    names: [Option<FixedName>; MAX_PLAYLISTS],
    len: usize,
}

impl PlaylistList {
    pub fn empty() -> Self {
        Self {
            names: [None; MAX_PLAYLISTS],
            len: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn get(&self, index: usize) -> Option<&FixedName> {
        if index >= self.len {
            return None;
        }
        self.names[index].as_ref()
    }

    fn push(&mut self, name: FixedName) {
        if self.len < MAX_PLAYLISTS {
            self.names[self.len] = Some(name);
            self.len += 1;
        }
    }

    fn sort(&mut self) {
        let mut compact = [FixedName::from_bytes(b"?").unwrap(); MAX_PLAYLISTS];
        for (slot, name) in compact
            .iter_mut()
            .take(self.len)
            .zip(self.names.iter().take(self.len))
        {
            if let Some(name) = name {
                *slot = *name;
            }
        }
        name::sort_names(&mut compact[..self.len]);
        for (slot, name) in self
            .names
            .iter_mut()
            .take(self.len)
            .zip(compact.iter().take(self.len))
        {
            *slot = Some(*name);
        }
    }
}

/// Enumerates `.txt` playlists in a chosen folder.
pub fn discover_playlists_in(directory: &[u8]) -> Result<PlaylistList, StorageError> {
    let storage = Storage::open();
    let c_path = path_to_cstring(directory);
    if !unsafe { sys::storage_dir_exists(storage.as_ptr(), c_path.as_c_ptr()) } {
        return Err(StorageError::MissingDirectory);
    }
    let file = File::default();
    if !unsafe { sys::storage_dir_open(file.as_ptr(), c_path.as_c_ptr()) } {
        return Err(StorageError::Read);
    }

    let mut list = PlaylistList {
        names: [None; MAX_PLAYLISTS],
        len: 0,
    };

    loop {
        let mut info = sys::FileInfo { flags: 0, size: 0 };
        let mut raw = [0 as c_char; DIRECTORY_NAME_LEN];
        if !unsafe {
            sys::storage_dir_read(
                file.as_ptr(),
                &mut info,
                raw.as_mut_ptr(),
                DIRECTORY_NAME_LEN as u16,
            )
        } {
            break;
        }
        if unsafe { sys::file_info_is_dir(&info) } {
            continue;
        }
        let file_name = unsafe { CStr::from_ptr(raw.as_ptr()) }.to_bytes();
        let Some(display) = name::playlist_display_name(file_name) else {
            continue;
        };
        if let Some(fixed) = FixedName::from_bytes(display) {
            list.push(fixed);
        }
    }

    list.sort();
    Ok(list)
}

/// Fixed-capacity list of subdirectories, sorted by name.
pub struct FolderList {
    names: [FixedName; MAX_FOLDERS],
    len: usize,
}

impl FolderList {
    pub fn empty() -> Self {
        Self {
            names: [FixedName::from_bytes(b"?").unwrap(); MAX_FOLDERS],
            len: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn get(&self, index: usize) -> Option<&FixedName> {
        if index >= self.len {
            None
        } else {
            Some(&self.names[index])
        }
    }

    fn push(&mut self, name: FixedName) {
        if self.len < MAX_FOLDERS {
            self.names[self.len] = name;
            self.len += 1;
        }
    }

    fn sort(&mut self) {
        name::sort_names(&mut self.names[..self.len]);
    }
}

/// Lists subdirectories of `directory`, sorted by name.
pub fn list_subdirectories(directory: &[u8]) -> Result<FolderList, StorageError> {
    let storage = Storage::open();
    let c_path = path_to_cstring(directory);
    if !unsafe { sys::storage_dir_exists(storage.as_ptr(), c_path.as_c_ptr()) } {
        return Err(StorageError::MissingDirectory);
    }
    let file = File::default();
    if !unsafe { sys::storage_dir_open(file.as_ptr(), c_path.as_c_ptr()) } {
        return Err(StorageError::Read);
    }

    let mut list = FolderList::empty();
    loop {
        let mut info = sys::FileInfo { flags: 0, size: 0 };
        let mut raw = [0 as c_char; DIRECTORY_NAME_LEN];
        if !unsafe {
            sys::storage_dir_read(
                file.as_ptr(),
                &mut info,
                raw.as_mut_ptr(),
                DIRECTORY_NAME_LEN as u16,
            )
        } {
            break;
        }
        if !unsafe { sys::file_info_is_dir(&info) } {
            continue;
        }
        let name = unsafe { CStr::from_ptr(raw.as_ptr()) }.to_bytes();
        if name == b"." || name == b".." || name.first() == Some(&b'.') {
            continue;
        }
        if let Some(fixed) = FixedName::from_bytes(name) {
            list.push(fixed);
        }
    }
    list.sort();
    Ok(list)
}

/// Builds the absolute `.txt` path in a chosen folder.
pub fn playlist_path_in(directory: &[u8], display_name: &FixedName) -> FuriString {
    let mut path = FuriString::new();
    append_bytes(&mut path, directory);
    if !directory.ends_with(b"/") {
        path.push('/');
    }
    append_bytes(&mut path, display_name.as_bytes());
    path.push_str(".txt");
    path
}

/// Loads the remembered playlist folder, or copies `default` if unset.
pub fn load_playlist_dir(default: &[u8], out: &mut [u8]) -> usize {
    let mut buffer = [0u8; 512];
    if let Ok(length) = read_file_into(SETTINGS_PATH.to_bytes(), &mut buffer) {
        for line in buffer[..length].split(|&byte| byte == b'\n') {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            if let Some(path) = line.strip_prefix(b"playlist_dir=") {
                if !path.is_empty() && path.len() <= out.len() {
                    out[..path.len()].copy_from_slice(path);
                    return path.len();
                }
            }
        }
    }
    let length = default.len().min(out.len());
    out[..length].copy_from_slice(&default[..length]);
    length
}

/// Persists the playlist folder choice.
pub fn save_playlist_dir(directory: &[u8]) -> Result<(), StorageError> {
    let mut file = File::create(SETTINGS_PATH).map_err(map_io)?;
    let mut text = FuriString::new();
    text.push_str("playlist_dir=");
    append_bytes(&mut text, directory);
    text.push('\n');
    file.write_all(text.as_c_str().to_bytes())
        .map_err(|_| StorageError::Write)?;
    if !unsafe { sys::storage_file_sync(file.as_ptr()) } {
        return Err(StorageError::Write);
    }
    Ok(())
}

/// Returns the parent directory, stopping at `/ext`.
pub fn parent_path(path: &[u8]) -> Option<&[u8]> {
    let trimmed = if path.len() > 1 && path.ends_with(b"/") {
        &path[..path.len() - 1]
    } else {
        path
    };
    if trimmed == b"/ext" || trimmed == b"/" {
        return None;
    }
    let index = trimmed.iter().rposition(|&byte| byte == b'/')?;
    if index == 0 {
        None
    } else {
        Some(&trimmed[..index])
    }
}

/// Joins a directory and an entry name into `out`.
pub fn join_path(directory: &[u8], entry: &[u8], out: &mut [u8]) -> Option<usize> {
    let needs_separator = !directory.ends_with(b"/");
    let length = directory.len() + if needs_separator { 1 } else { 0 } + entry.len();
    if length > out.len() {
        return None;
    }
    out[..directory.len()].copy_from_slice(directory);
    let mut position = directory.len();
    if needs_separator {
        out[position] = b'/';
        position += 1;
    }
    out[position..position + entry.len()].copy_from_slice(entry);
    Some(length)
}

fn path_to_cstring(path: &[u8]) -> FuriString {
    let mut text = FuriString::new();
    append_bytes(&mut text, path);
    text
}

/// Appends `bytes` to `target`. ASCII bytes (the upstream-valid case for
/// paths, names and labels) are copied in one bulk call; other bytes keep the
/// per-character encoding so the result matches `push(byte as char)` exactly.
pub(crate) fn append_bytes(target: &mut FuriString, bytes: &[u8]) {
    if bytes.iter().all(|&byte| byte.is_ascii()) {
        if let Ok(text) = core::str::from_utf8(bytes) {
            target.push_str(text);
            return;
        }
    }
    for &byte in bytes {
        target.push(byte as char);
    }
}

/// Builds a display name from the session's absolute playlist path.
pub fn display_name_from_path(path: &[u8]) -> Option<FixedName> {
    let file_name = path.rsplit(|&byte| byte == b'/').next()?;
    let display = name::playlist_display_name(file_name)?;
    FixedName::from_bytes(display)
}

/// Streaming line source over a Flipper file.
pub struct FileLineSource {
    file: File,
    buffer: [u8; 512],
    position: usize,
    filled: usize,
    eof: bool,
}

impl FileLineSource {
    fn new(file: File) -> Self {
        Self {
            file,
            buffer: [0u8; 512],
            position: 0,
            filled: 0,
            eof: false,
        }
    }

    fn refill(&mut self) -> Result<bool, LineReadError> {
        if self.eof {
            return Ok(false);
        }
        match self.file.read(&mut self.buffer) {
            Ok(0) => {
                self.eof = true;
                Ok(false)
            }
            Ok(count) => {
                self.position = 0;
                self.filled = count;
                Ok(true)
            }
            Err(_) => Err(LineReadError::Io),
        }
    }
}

impl LineSource for FileLineSource {
    fn next_line(&mut self, out: &mut [u8]) -> Result<Option<usize>, LineReadError> {
        let mut length = 0usize;
        let mut seen = false;

        loop {
            if self.position == self.filled && !self.refill()? {
                if !seen {
                    return Ok(None);
                }
                break;
            }
            let byte = self.buffer[self.position];
            self.position += 1;
            seen = true;
            if byte == b'\n' {
                break;
            }
            if length == out.len() {
                return Err(LineReadError::LineTooLong);
            }
            out[length] = byte;
            length += 1;
        }

        if length > 0 && out[length - 1] == b'\r' {
            length -= 1;
        }
        Ok(Some(length))
    }
}

/// Opens a playlist by its stored absolute path.
///
/// The returned stream can serve multiple entry reads; it must be dropped
/// before the same path is opened again.
pub fn open_playlist_path(path: &[u8]) -> Result<FileLineSource, StorageError> {
    let mut c_path = FuriString::new();
    append_bytes(&mut c_path, path);
    let file = File::open(c_path.as_c_str()).map_err(map_io)?;
    Ok(FileLineSource::new(file))
}

/// Playlist open, read or parse failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaylistLoadError {
    Missing,
    Read,
    Parse(PlaylistError),
}

impl From<StorageError> for PlaylistLoadError {
    fn from(error: StorageError) -> Self {
        match error {
            StorageError::MissingFile | StorageError::MissingDirectory => Self::Missing,
            _ => Self::Read,
        }
    }
}

/// Scans a playlist by path, validating every entry.
pub fn scan_playlist_path(path: &[u8]) -> Result<PlaylistStats, PlaylistLoadError> {
    let mut source = open_playlist_path(path).map_err(PlaylistLoadError::from)?;
    let mut line = [0u8; playlist::LINE_BUFFER_SIZE];
    playlist::scan(&mut source, &mut line).map_err(PlaylistLoadError::Parse)
}

/// Reads one URL from a stored playlist path.
///
/// Re-opens the playlist and streams from the first line. Prefer
/// [`playlist_entry_from_source`] when the file can stay open between reads.
#[allow(dead_code)]
pub fn playlist_entry_path(
    path: &[u8],
    index: u32,
    out: &mut [u8],
) -> Result<usize, PlaylistLoadError> {
    let mut source = open_playlist_path(path).map_err(PlaylistLoadError::from)?;
    let mut line = [0u8; playlist::LINE_BUFFER_SIZE];
    playlist::entry_at(&mut source, index, &mut line, out).map_err(PlaylistLoadError::Parse)
}

/// Reads one URL from an already-open playlist stream.
///
/// `index` is zero-based counting from the stream's *current* position, which
/// lets a caller whose playlist file stays open between productions consume
/// later entries without re-opening or re-reading the file.
pub fn playlist_entry_from_source(
    source: &mut FileLineSource,
    index: u32,
    out: &mut [u8],
) -> Result<usize, PlaylistLoadError> {
    let mut line = [0u8; playlist::LINE_BUFFER_SIZE];
    playlist::entry_at(source, index, &mut line, out).map_err(PlaylistLoadError::Parse)
}

/// Result of trying to load a session from storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadedSession {
    Missing,
    Corrupt,
    ReadError,
    Loaded,
}

/// Reads the newest valid session record, if any.
pub fn load_session(session_out: &mut Session) -> LoadedSession {
    let mut buffer = [0u8; MAX_SESSION_LEN];

    match read_session_file(SESSION_PATH, &mut buffer) {
        Ok(Some(length)) => match Session::parse(&buffer[..length]) {
            Ok(parsed) => {
                *session_out = parsed;
                LoadedSession::Loaded
            }
            Err(_) => load_backup(session_out, true),
        },
        Ok(None) => load_backup(session_out, false),
        Err(_) => load_backup(session_out, true),
    }
}

fn load_backup(session_out: &mut Session, primary_problem: bool) -> LoadedSession {
    let mut buffer = [0u8; MAX_SESSION_LEN];
    match read_session_file(SESSION_BAK_PATH, &mut buffer) {
        Ok(Some(length)) => match Session::parse(&buffer[..length]) {
            Ok(parsed) => {
                *session_out = parsed;
                LoadedSession::Loaded
            }
            Err(_) => LoadedSession::Corrupt,
        },
        Ok(None) => {
            if primary_problem {
                LoadedSession::Corrupt
            } else {
                LoadedSession::Missing
            }
        }
        Err(_) => LoadedSession::ReadError,
    }
}

fn read_session_file(path: &CStr, buffer: &mut [u8]) -> Result<Option<usize>, StorageError> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(IoError::NotExists) => return Ok(None),
        Err(_) => return Err(StorageError::Read),
    };
    let mut total = 0usize;
    loop {
        if total == buffer.len() {
            // Anything longer than the maximum is not a valid session.
            return Err(StorageError::Corrupt);
        }
        match file.read(&mut buffer[total..]) {
            Ok(0) => return Ok(Some(total)),
            Ok(count) => total += count,
            Err(_) => return Err(StorageError::Read),
        }
    }
}

/// Persists a session with temporary-file replacement and one backup.
pub fn save_session(session: &Session) -> Result<(), StorageError> {
    let mut buffer = [0u8; MAX_SESSION_LEN];
    let length = session
        .serialize(&mut buffer)
        .map_err(|_| StorageError::Write)?;

    {
        let mut file = File::create(SESSION_TMP_PATH).map_err(|_| StorageError::Write)?;
        file.write_all(&buffer[..length])
            .map_err(|_| StorageError::Write)?;
        file.flush().map_err(|_| StorageError::Write)?;
        if !unsafe { sys::storage_file_sync(file.as_ptr()) } {
            return Err(StorageError::Write);
        }
    }

    let storage = Storage::open();
    let mut info = sys::FileInfo { flags: 0, size: 0 };
    let exists =
        unsafe { sys::storage_common_stat(storage.as_ptr(), SESSION_PATH.as_ptr(), &mut info) }
            == sys::FSE_OK;
    if exists {
        let _ = unsafe { sys::storage_common_remove(storage.as_ptr(), SESSION_BAK_PATH.as_ptr()) };
        let status = unsafe {
            sys::storage_common_rename(
                storage.as_ptr(),
                SESSION_PATH.as_ptr(),
                SESSION_BAK_PATH.as_ptr(),
            )
        };
        if status != sys::FSE_OK {
            let _ =
                unsafe { sys::storage_common_remove(storage.as_ptr(), SESSION_TMP_PATH.as_ptr()) };
            return Err(StorageError::Write);
        }
    }

    let status = unsafe {
        sys::storage_common_rename(
            storage.as_ptr(),
            SESSION_TMP_PATH.as_ptr(),
            SESSION_PATH.as_ptr(),
        )
    };
    if status != sys::FSE_OK {
        return Err(StorageError::Write);
    }
    Ok(())
}

/// Deletes the session and its backup.
pub fn clear_session() -> Result<(), StorageError> {
    let storage = Storage::open();
    for path in [SESSION_PATH, SESSION_TMP_PATH, SESSION_BAK_PATH] {
        let mut info = sys::FileInfo { flags: 0, size: 0 };
        if unsafe { sys::storage_common_stat(storage.as_ptr(), path.as_ptr(), &mut info) }
            == sys::FSE_OK
        {
            let status = unsafe { sys::storage_common_remove(storage.as_ptr(), path.as_ptr()) };
            if status != sys::FSE_OK {
                return Err(StorageError::Write);
            }
        }
    }
    Ok(())
}

/// Reads a small file into `buffer` and returns its length.
pub fn read_file_into(path: &[u8], buffer: &mut [u8]) -> Result<usize, StorageError> {
    let mut c_path = FuriString::new();
    append_bytes(&mut c_path, path);
    let mut file = File::open(c_path.as_c_str()).map_err(map_io)?;
    let mut total = 0usize;
    loop {
        if total == buffer.len() {
            return Err(StorageError::Read);
        }
        match file.read(&mut buffer[total..]) {
            Ok(0) => return Ok(total),
            Ok(count) => total += count,
            Err(_) => return Err(StorageError::Read),
        }
    }
}

/// Appends one line to the debug trace file, ignoring all errors.
pub fn trace(message: &str) {
    let mut line = FuriString::new();
    line.push_str(message);
    line.push('\n');
    if let Ok(mut file) = OpenOptions::new()
        .write(true)
        .open_append(true)
        .open(c"/ext/apps_data/nfc-bulk-url-writer/trace.log")
    {
        let _ = file.write_all(line.as_c_str().to_bytes());
    }
}

/// Returns true when the stored path exists as a file.
pub fn path_exists(path: &[u8]) -> bool {
    let mut c_path = FuriString::new();
    append_bytes(&mut c_path, path);
    let storage = Storage::open();
    let mut info = sys::FileInfo { flags: 0, size: 0 };
    (unsafe { sys::storage_common_stat(storage.as_ptr(), c_path.as_c_ptr(), &mut info) })
        == sys::FSE_OK
}

fn map_io(error: IoError) -> StorageError {
    match error {
        IoError::NotExists => StorageError::MissingFile,
        _ => StorageError::Read,
    }
}
