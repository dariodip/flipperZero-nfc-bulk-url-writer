//! NTAG213/215/216 identification, validation and capacity rules.
//!
//! The data model mirrors the pages read from a tag. The validation pipeline is
//! deliberately conservative: anything unusual is rejected rather than written.

use crate::errors::TagError;

/// Number of pages the application can hold (NTAG216 has 231).
pub const MAX_PAGES: usize = 231;
/// One tag page is four bytes.
pub const PAGE_SIZE: usize = 4;
/// Size of the raw page array.
pub const MAX_TAG_BYTES: usize = MAX_PAGES * PAGE_SIZE;
/// Size of GET_VERSION response.
pub const VERSION_LEN: usize = 8;
/// Maximum UID length for ISO14443-3A tags.
pub const MAX_UID_LEN: usize = 10;

/// Supported tag models.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagModel {
    Ntag213,
    Ntag215,
    Ntag216,
}

impl TagModel {
    /// Identifies a model from the GET_VERSION storage-size byte.
    pub fn from_storage_size(storage_size: u8) -> Option<Self> {
        match storage_size {
            0x0F => Some(Self::Ntag213),
            0x11 => Some(Self::Ntag215),
            0x13 => Some(Self::Ntag216),
            _ => None,
        }
    }

    /// GET_VERSION storage-size byte reported by this model.
    pub fn storage_size(self) -> u8 {
        match self {
            Self::Ntag213 => 0x0F,
            Self::Ntag215 => 0x11,
            Self::Ntag216 => 0x13,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Ntag213 => "NTAG213",
            Self::Ntag215 => "NTAG215",
            Self::Ntag216 => "NTAG216",
        }
    }

    /// Value of Capability Container byte 2 for this model.
    pub fn cc_size_byte(self) -> u8 {
        match self {
            Self::Ntag213 => 0x12,
            Self::Ntag215 => 0x3E,
            Self::Ntag216 => 0x6D,
        }
    }

    /// NDEF memory area derived from the Capability Container (byte 2 * 8).
    ///
    /// This is intentionally smaller than the nominal user memory for NTAG215
    /// and NTAG216.
    pub fn ndef_memory(self) -> usize {
        self.cc_size_byte() as usize * 8
    }

    /// Nominal user memory (pages 4 through the page before the dynamic lock page).
    pub fn user_memory(self) -> usize {
        (self.user_page_count() as usize) * PAGE_SIZE
    }

    /// Total number of pages reported by GET_VERSION.
    pub fn page_count(self) -> u16 {
        match self {
            Self::Ntag213 => 45,
            Self::Ntag215 => 135,
            Self::Ntag216 => 231,
        }
    }

    /// First configuration page.
    pub fn config_page(self) -> u16 {
        match self {
            Self::Ntag213 => 41,
            Self::Ntag215 => 131,
            Self::Ntag216 => 227,
        }
    }

    /// Page containing the dynamic lock bits (config page - 1).
    pub fn dynamic_lock_page(self) -> u16 {
        self.config_page() - 1
    }

    /// Last user-memory page.
    pub fn last_user_page(self) -> u16 {
        self.dynamic_lock_page() - 1
    }

    /// Number of user-memory pages.
    pub fn user_page_count(self) -> u16 {
        self.last_user_page() - 4 + 1
    }

    /// First page of the NDEF area.
    pub const fn first_user_page() -> u16 {
        4
    }
}

/// Raw pages read from a tag plus its GET_VERSION response and UID.
#[derive(Clone)]
pub struct TagSnapshot {
    pages: [u8; MAX_TAG_BYTES],
    pub pages_read: u16,
    pub version: [u8; VERSION_LEN],
    pub uid: [u8; MAX_UID_LEN],
    pub uid_len: u8,
}

impl Default for TagSnapshot {
    fn default() -> Self {
        Self::new()
    }
}

impl TagSnapshot {
    pub fn new() -> Self {
        Self {
            pages: [0u8; MAX_TAG_BYTES],
            pages_read: 0,
            version: [0u8; VERSION_LEN],
            uid: [0u8; MAX_UID_LEN],
            uid_len: 0,
        }
    }

    /// Copies a page into the snapshot. Returns false for out-of-range pages.
    pub fn set_page(&mut self, page: u16, data: [u8; PAGE_SIZE]) -> bool {
        let page = page as usize;
        if page >= MAX_PAGES {
            return false;
        }
        let offset = page * PAGE_SIZE;
        self.pages[offset..offset + PAGE_SIZE].copy_from_slice(&data);
        true
    }

    pub fn set_pages_read(&mut self, count: u16) {
        self.pages_read = count;
    }

    pub fn set_version(&mut self, version: [u8; VERSION_LEN]) {
        self.version = version;
    }

    pub fn set_uid(&mut self, uid: &[u8]) {
        let length = uid.len().min(MAX_UID_LEN);
        self.uid[..length].copy_from_slice(&uid[..length]);
        self.uid_len = length as u8;
    }

    pub fn page(&self, page: u16) -> Option<[u8; PAGE_SIZE]> {
        if page as usize >= MAX_PAGES {
            return None;
        }
        let offset = page as usize * PAGE_SIZE;
        let mut out = [0u8; PAGE_SIZE];
        out.copy_from_slice(&self.pages[offset..offset + PAGE_SIZE]);
        Some(out)
    }

    pub fn uid(&self) -> &[u8] {
        &self.uid[..self.uid_len as usize]
    }

    /// Identifies the model from GET_VERSION, never from the UID.
    pub fn model(&self) -> Option<TagModel> {
        TagModel::from_storage_size(self.version[6])
    }
}

/// Validated tag information returned by [`validate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TagInfo {
    pub model: TagModel,
    pub ndef_memory: usize,
    pub user_memory: usize,
    /// Exact TLV size the tag has room for.
    pub tlv_size: usize,
}

/// Validates a tag snapshot and checks that `required_tlv_size` fits.
///
/// The snapshot must contain at least the model's configuration pages.
pub fn validate(snapshot: &TagSnapshot, required_tlv_size: usize) -> Result<TagInfo, TagError> {
    let model = snapshot.model().ok_or(TagError::UnsupportedTag)?;

    if snapshot.pages_read < model.config_page() + 4 {
        return Err(TagError::IncompleteDump);
    }

    validate_metadata(snapshot, required_tlv_size)
}

/// Validates model, Capability Container, locks, protection and capacity.
///
/// Unlike [`validate`], this does not require every user page to have been
/// read. It is used by the worker, which reads only the metadata pages and the
/// pages it is about to write.
pub fn validate_metadata(
    snapshot: &TagSnapshot,
    required_tlv_size: usize,
) -> Result<TagInfo, TagError> {
    let model = snapshot.model().ok_or(TagError::UnsupportedTag)?;

    validate_capability_container(snapshot, model)?;
    validate_locks(snapshot, model)?;
    validate_protection(snapshot, model)?;

    let ndef_memory = model.ndef_memory();
    if required_tlv_size > ndef_memory {
        return Err(TagError::InsufficientCapacity {
            required: required_tlv_size,
            available: ndef_memory,
        });
    }

    Ok(TagInfo {
        model,
        ndef_memory,
        user_memory: model.user_memory(),
        tlv_size: required_tlv_size,
    })
}

/// Validates page 3 as a Type 2 Tag Capability Container.
pub fn validate_capability_container(
    snapshot: &TagSnapshot,
    model: TagModel,
) -> Result<(), TagError> {
    let cc = snapshot
        .page(3)
        .ok_or(TagError::InvalidCapabilityContainer)?;
    if cc[0] != 0xE1 || cc[1] != 0x10 {
        return Err(TagError::InvalidCapabilityContainer);
    }
    if cc[2] != model.cc_size_byte() {
        return Err(TagError::CapabilityContainerMismatch);
    }
    // Bits 7-4 are write access and bits 3-0 are read access. Zero means free.
    if cc[3] != 0x00 {
        return Err(TagError::ReadOnly);
    }
    Ok(())
}

/// Validates static and dynamic lock bytes.
pub fn validate_locks(snapshot: &TagSnapshot, model: TagModel) -> Result<(), TagError> {
    let static_lock = snapshot.page(2).ok_or(TagError::IncompleteDump)?;
    if static_lock[2] != 0 || static_lock[3] != 0 {
        return Err(TagError::Locked);
    }

    let dynamic = snapshot
        .page(model.dynamic_lock_page())
        .ok_or(TagError::IncompleteDump)?;
    if dynamic[0] != 0 || dynamic[1] != 0 || dynamic[2] != 0 {
        return Err(TagError::Locked);
    }
    Ok(())
}

/// Validates `PROT` and `AUTH0` protection for the user area.
pub fn validate_protection(snapshot: &TagSnapshot, model: TagModel) -> Result<(), TagError> {
    let config = snapshot
        .page(model.config_page())
        .ok_or(TagError::IncompleteDump)?;
    let access = snapshot
        .page(model.config_page() + 1)
        .ok_or(TagError::IncompleteDump)?;

    let auth0 = config[3];
    let access_byte = access[0];

    // PROT (bit 7) makes the whole user memory read-only.
    if access_byte & 0x80 != 0 {
        return Err(TagError::ReadOnly);
    }

    // AUTH0 is the first page that requires authentication. 0xFF disables it.
    if (auth0 as u16) <= model.last_user_page() {
        return Err(TagError::PasswordProtected);
    }

    Ok(())
}

/// Checks only capacity using the model's CC-derived NDEF memory.
pub fn check_capacity(model: TagModel, required_tlv_size: usize) -> Result<(), TagError> {
    if required_tlv_size > model.ndef_memory() {
        return Err(TagError::InsufficientCapacity {
            required: required_tlv_size,
            available: model.ndef_memory(),
        });
    }
    Ok(())
}

/// Builds a factory-like snapshot for tests.
#[cfg(test)]
pub fn factory_snapshot(model: TagModel) -> TagSnapshot {
    let mut snapshot = TagSnapshot::new();
    snapshot.set_page(0, [0x04, 0x12, 0x34, 0x56]);
    snapshot.set_page(1, [0x78, 0x9A, 0xBC, 0xDE]);
    snapshot.set_page(2, [0x00, 0x00, 0x00, 0x00]);
    snapshot.set_page(3, [0xE1, 0x10, model.cc_size_byte(), 0x00]);
    snapshot.set_page(model.dynamic_lock_page(), [0, 0, 0, 0]);

    let mut config_page = [0u8; 4];
    config_page[3] = 0xFF; // AUTH0 disabled
    snapshot.set_page(model.config_page(), config_page);
    snapshot.set_page(model.config_page() + 1, [0, 0, 0, 0]);
    snapshot.set_page(model.config_page() + 2, [0, 0, 0, 0]);
    snapshot.set_page(model.config_page() + 3, [0, 0, 0, 0]);

    let mut version = [0u8; VERSION_LEN];
    version[0] = 0x00;
    version[1] = 0x04;
    version[2] = 0x04;
    version[3] = 0x02;
    version[4] = 0x01;
    version[5] = 0x00;
    version[6] = match model {
        TagModel::Ntag213 => 0x0F,
        TagModel::Ntag215 => 0x11,
        TagModel::Ntag216 => 0x13,
    };
    version[7] = 0x03;
    snapshot.set_version(version);
    snapshot.set_pages_read(model.page_count());
    snapshot
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_supported_models_from_version() {
        assert_eq!(TagModel::from_storage_size(0x0F), Some(TagModel::Ntag213));
        assert_eq!(TagModel::from_storage_size(0x11), Some(TagModel::Ntag215));
        assert_eq!(TagModel::from_storage_size(0x13), Some(TagModel::Ntag216));
        for unsupported in [0x00, 0x0E, 0x10, 0x12, 0x14, 0xFF] {
            assert_eq!(TagModel::from_storage_size(unsupported), None);
        }
    }

    #[test]
    fn storage_size_round_trips() {
        for model in [TagModel::Ntag213, TagModel::Ntag215, TagModel::Ntag216] {
            assert_eq!(
                TagModel::from_storage_size(model.storage_size()),
                Some(model)
            );
        }
    }

    #[test]
    fn model_capacities_match_capability_container() {
        assert_eq!(TagModel::Ntag213.ndef_memory(), 144);
        assert_eq!(TagModel::Ntag215.ndef_memory(), 496);
        assert_eq!(TagModel::Ntag216.ndef_memory(), 872);
        assert_eq!(TagModel::Ntag213.user_memory(), 144);
        assert_eq!(TagModel::Ntag215.user_memory(), 504);
        assert_eq!(TagModel::Ntag216.user_memory(), 888);
    }

    #[test]
    fn accepts_factory_snapshots_for_all_models() {
        for model in [TagModel::Ntag213, TagModel::Ntag215, TagModel::Ntag216] {
            let snapshot = factory_snapshot(model);
            assert_eq!(validate(&snapshot, 100).unwrap().model, model);
            assert_eq!(
                validate(&snapshot, 100).unwrap().ndef_memory,
                model.ndef_memory()
            );
        }
    }

    #[test]
    fn rejects_unsupported_version() {
        let mut snapshot = factory_snapshot(TagModel::Ntag215);
        snapshot.version[6] = 0x10;
        assert_eq!(validate(&snapshot, 10), Err(TagError::UnsupportedTag));
    }

    #[test]
    fn rejects_incomplete_dump() {
        let mut snapshot = factory_snapshot(TagModel::Ntag215);
        snapshot.set_pages_read(100);
        assert_eq!(validate(&snapshot, 10), Err(TagError::IncompleteDump));
    }

    #[test]
    fn validates_capability_container() {
        assert!(
            validate_capability_container(&factory_snapshot(TagModel::Ntag215), TagModel::Ntag215)
                .is_ok()
        );

        let mut bad_magic = factory_snapshot(TagModel::Ntag215);
        bad_magic.set_page(3, [0xE2, 0x10, 0x3E, 0x00]);
        assert_eq!(
            validate_capability_container(&bad_magic, TagModel::Ntag215),
            Err(TagError::InvalidCapabilityContainer)
        );

        let mut bad_version = factory_snapshot(TagModel::Ntag215);
        bad_version.set_page(3, [0xE1, 0x20, 0x3E, 0x00]);
        assert_eq!(
            validate_capability_container(&bad_version, TagModel::Ntag215),
            Err(TagError::InvalidCapabilityContainer)
        );

        let mut mismatch = factory_snapshot(TagModel::Ntag215);
        mismatch.set_page(3, [0xE1, 0x10, 0x12, 0x00]);
        assert_eq!(
            validate_capability_container(&mismatch, TagModel::Ntag215),
            Err(TagError::CapabilityContainerMismatch)
        );

        let mut read_only = factory_snapshot(TagModel::Ntag215);
        read_only.set_page(3, [0xE1, 0x10, 0x3E, 0x0F]);
        assert_eq!(
            validate_capability_container(&read_only, TagModel::Ntag215),
            Err(TagError::ReadOnly)
        );
    }

    #[test]
    fn metadata_validation_does_not_require_user_pages() {
        let mut snapshot = factory_snapshot(TagModel::Ntag215);
        snapshot.set_pages_read(12);
        assert!(validate_metadata(&snapshot, 100).is_ok());
        assert_eq!(validate(&snapshot, 100), Err(TagError::IncompleteDump));
    }

    #[test]
    fn rejects_static_and_dynamic_locks() {
        let mut static_locked = factory_snapshot(TagModel::Ntag215);
        static_locked.set_page(2, [0, 0, 0x01, 0]);
        assert_eq!(
            validate_locks(&static_locked, TagModel::Ntag215),
            Err(TagError::Locked)
        );

        let mut dynamic_locked = factory_snapshot(TagModel::Ntag215);
        dynamic_locked.set_page(TagModel::Ntag215.dynamic_lock_page(), [0, 0, 0x01, 0]);
        assert_eq!(
            validate_locks(&dynamic_locked, TagModel::Ntag215),
            Err(TagError::Locked)
        );
    }

    #[test]
    fn rejects_protection_and_password() {
        let mut read_only = factory_snapshot(TagModel::Ntag215);
        read_only.set_page(TagModel::Ntag215.config_page() + 1, [0x80, 0, 0, 0]);
        assert_eq!(
            validate_protection(&read_only, TagModel::Ntag215),
            Err(TagError::ReadOnly)
        );

        let mut password = factory_snapshot(TagModel::Ntag215);
        let mut config = password.page(TagModel::Ntag215.config_page()).unwrap();
        config[3] = 4;
        password.set_page(TagModel::Ntag215.config_page(), config);
        assert_eq!(
            validate_protection(&password, TagModel::Ntag215),
            Err(TagError::PasswordProtected)
        );
    }

    #[test]
    fn capacity_uses_cc_derived_ndef_memory() {
        assert!(check_capacity(TagModel::Ntag215, 496).is_ok());
        assert_eq!(
            check_capacity(TagModel::Ntag215, 497),
            Err(TagError::InsufficientCapacity {
                required: 497,
                available: 496
            })
        );
        assert!(check_capacity(TagModel::Ntag216, 872).is_ok());
        assert_eq!(
            check_capacity(TagModel::Ntag216, 873),
            Err(TagError::InsufficientCapacity {
                required: 873,
                available: 872
            })
        );
        assert!(check_capacity(TagModel::Ntag213, 144).is_ok());
        assert_eq!(
            check_capacity(TagModel::Ntag213, 145),
            Err(TagError::InsufficientCapacity {
                required: 145,
                available: 144
            })
        );
    }

    #[test]
    fn snapshot_round_trips_pages_and_uid() {
        let mut snapshot = TagSnapshot::new();
        assert!(snapshot.set_page(4, [1, 2, 3, 4]));
        assert_eq!(snapshot.page(4), Some([1, 2, 3, 4]));
        assert!(!snapshot.set_page(MAX_PAGES as u16, [0; 4]));
        snapshot.set_uid(&[0x04, 0xA1, 0xB2, 0xC3, 0xD4, 0xE5, 0xF6]);
        assert_eq!(snapshot.uid(), &[0x04, 0xA1, 0xB2, 0xC3, 0xD4, 0xE5, 0xF6]);
    }
}
