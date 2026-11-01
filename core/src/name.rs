//! Fixed-size playlist names and deterministic ordering.
//!
//! Directory listing, filtering and sorting are kept here so they can be
//! tested on the host without the Flipper SDK.

/// Maximum stored playlist name length.
pub const MAX_NAME_LEN: usize = 64;

/// A fixed-capacity file name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FixedName {
    bytes: [u8; MAX_NAME_LEN],
    len: u8,
}

impl FixedName {
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.is_empty() || bytes.len() > MAX_NAME_LEN {
            return None;
        }
        let mut out = [0u8; MAX_NAME_LEN];
        out[..bytes.len()].copy_from_slice(bytes);
        Some(Self {
            bytes: out,
            len: bytes.len() as u8,
        })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }

    pub fn as_str(&self) -> Option<&str> {
        core::str::from_utf8(self.as_bytes()).ok()
    }
}

/// Returns the display name for a `.txt` playlist file, without the extension.
///
/// Names containing a path separator or an empty base are rejected.
pub fn playlist_display_name(file_name: &[u8]) -> Option<&[u8]> {
    if file_name.len() <= 4 || file_name.contains(&b'/') || file_name.contains(&b'\\') {
        return None;
    }
    let (base, extension) = file_name.split_at(file_name.len() - 4);
    if !extension.eq_ignore_ascii_case(b".txt") {
        return None;
    }
    Some(base)
}

/// Sorts names ascending by display-name bytes.
pub fn sort_names(names: &mut [FixedName]) {
    for index in 1..names.len() {
        let mut position = index;
        while position > 0 && names[position - 1].as_bytes() > names[position].as_bytes() {
            names.swap(position - 1, position);
            position -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;

    fn name(bytes: &[u8]) -> FixedName {
        FixedName::from_bytes(bytes).unwrap()
    }

    #[test]
    fn filters_txt_files() {
        assert_eq!(
            playlist_display_name(b"batch001.txt"),
            Some(&b"batch001"[..])
        );
        assert_eq!(
            playlist_display_name(b"batch002.TXT"),
            Some(&b"batch002"[..])
        );
        assert_eq!(playlist_display_name(b"notes.md"), None);
        assert_eq!(playlist_display_name(b".txt"), None);
        assert_eq!(playlist_display_name(b"txt"), None);
        assert_eq!(playlist_display_name(b"dir/batch.txt"), None);
    }

    #[test]
    fn sorts_deterministically() {
        let mut names = [
            name(b"batch002"),
            name(b"batch001"),
            name(b"zeta"),
            name(b"alpha"),
        ];
        sort_names(&mut names);
        let sorted: Vec<&[u8]> = names.iter().map(FixedName::as_bytes).collect();
        assert_eq!(
            sorted,
            std::vec![&b"alpha"[..], b"batch001", b"batch002", b"zeta"]
        );
    }

    #[test]
    fn rejects_overlong_and_empty_names() {
        assert!(FixedName::from_bytes(b"").is_none());
        let long = [b'a'; MAX_NAME_LEN + 1];
        assert!(FixedName::from_bytes(&long).is_none());
        assert!(FixedName::from_bytes(&long[..MAX_NAME_LEN]).is_some());
    }

    #[test]
    fn sorts_short_slices() {
        let mut empty: [FixedName; 0] = [];
        sort_names(&mut empty);
        let mut one = [name(b"only")];
        sort_names(&mut one);
        assert_eq!(one[0].as_bytes(), b"only");
    }
}
