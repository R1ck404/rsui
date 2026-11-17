//! Immutable packed bytecode storage. Mapping is an optional local-file fast
//! path; failure retains the original owned read and all code-cache validation.

use std::fs::File;
use std::io;
use std::ops::Deref;

pub(crate) enum CodeBytes {
    Owned(Vec<u8>),
    #[cfg(windows)]
    Mapped(windows::Mapping),
}

impl From<Vec<u8>> for CodeBytes {
    fn from(bytes: Vec<u8>) -> Self {
        Self::Owned(bytes)
    }
}

impl Deref for CodeBytes {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            Self::Owned(bytes) => bytes,
            #[cfg(windows)]
            Self::Mapped(mapping) => mapping.bytes(),
        }
    }
}

impl CodeBytes {
    pub(crate) fn read(file: &File, offset: u64, size: u64) -> io::Result<Self> {
        Self::read_mode(
            file,
            offset,
            size,
            std::env::var_os("RSUI_NO_CODE_MAP").is_none_or(|value| value != "1"),
        )
    }

    fn read_mode(file: &File, offset: u64, size: u64, allow_mapping: bool) -> io::Result<Self> {
        let len = checked_range(offset, size, file.metadata()?.len())?;
        #[cfg(windows)]
        if allow_mapping && len > 0 {
            if let Ok(mapping) = windows::Mapping::new(file, offset, size) {
                return Ok(Self::Mapped(mapping));
            }
        }
        #[cfg(not(windows))]
        let _ = allow_mapping;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(len)
            .map_err(|_| io::Error::other("packed code allocation failed"))?;
        bytes.resize(len, 0);
        #[cfg(windows)]
        crate::packed::read_at(file, &mut bytes, offset)?;
        #[cfg(not(windows))]
        {
            use std::io::{Read, Seek, SeekFrom};
            let mut reader = file.try_clone()?;
            reader.seek(SeekFrom::Start(offset))?;
            reader.read_exact(&mut bytes)?;
        }
        Ok(Self::Owned(bytes))
    }
}

fn checked_range(offset: u64, size: u64, file_size: u64) -> io::Result<usize> {
    let end = offset
        .checked_add(size)
        .ok_or_else(|| io::Error::other("packed code range overflow"))?;
    let len = usize::try_from(size)
        .ok()
        .filter(|len| *len <= isize::MAX as usize)
        .ok_or_else(|| io::Error::other("packed code range is too large"))?;
    if end > file_size {
        return Err(io::ErrorKind::UnexpectedEof.into());
    }
    Ok(len)
}

#[cfg(windows)]
// Confine the OS mapping boundary; all consumers receive an ordinary immutable
// slice owned by the block, without exposing raw handles or mutable pointers.
#[allow(unsafe_code)]
mod windows {
    use super::*;
    use std::ffi::c_void;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::ptr::NonNull;

    type Handle = *mut c_void;
    const GENERIC_READ: u32 = 0x8000_0000;
    const SHARE_READ_DELETE: u32 = 1 | 4;
    const PAGE_READONLY: u32 = 2;
    const FILE_MAP_READ: u32 = 4;

    #[repr(C)]
    struct SystemInfo {
        architecture: u32,
        page_size: u32,
        minimum_address: *mut c_void,
        maximum_address: *mut c_void,
        active_processor_mask: usize,
        processors: u32,
        processor_type: u32,
        allocation_granularity: u32,
        processor_level: u16,
        processor_revision: u16,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn ReOpenFile(file: Handle, access: u32, share: u32, flags: u32) -> Handle;
        fn GetSystemInfo(info: *mut SystemInfo);
        fn GetFinalPathNameByHandleW(file: Handle, path: *mut u16, size: u32, flags: u32) -> u32;
        fn GetDriveTypeW(root: *const u16) -> u32;
        fn CreateFileMappingW(
            file: isize,
            security: *const c_void,
            protection: u32,
            high: u32,
            low: u32,
            name: *const u16,
        ) -> isize;
        fn MapViewOfFile(
            mapping: isize,
            access: u32,
            high: u32,
            low: u32,
            size: usize,
        ) -> *mut c_void;
        fn UnmapViewOfFile(view: *const c_void) -> i32;
    }

    pub(crate) struct Mapping {
        base: NonNull<u8>,
        delta: usize,
        len: usize,
        // Dropped after unmapping, with the strict share lock held to the end.
        _mapping: OwnedHandle,
        _file: File,
    }

    impl Mapping {
        pub(super) fn new(file: &File, offset: u64, size: u64) -> io::Result<Self> {
            // Reopen the SAME object, never its path: an updater can rename the
            // running executable and put another file at that pathname.
            // Omitting SHARE_WRITE must reject existing writers/writable maps
            // and denies new ones for the strict handle's complete lifetime.
            let raw =
                unsafe { ReOpenFile(file.as_raw_handle(), GENERIC_READ, SHARE_READ_DELETE, 0) };
            if raw.is_null() || raw == -1isize as Handle {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: ReOpenFile returned a new owned handle, transferred once.
            let strict = unsafe { File::from_raw_handle(raw) };
            if !local_disk(&strict) {
                return Err(io::Error::other(
                    "packed code mapping needs a certified local disk",
                ));
            }
            let len = checked_range(offset, size, strict.metadata()?.len())?;
            if len == 0 {
                return Err(io::Error::other("empty packed code mapping"));
            }
            let mut info = std::mem::MaybeUninit::<SystemInfo>::uninit();
            // SAFETY: the API initializes this correctly laid-out plain struct.
            unsafe {
                GetSystemInfo(info.as_mut_ptr());
            }
            let granularity = u64::from(unsafe { info.assume_init() }.allocation_granularity);
            if granularity == 0 {
                return Err(io::Error::other("unknown mapping granularity"));
            }
            let aligned = offset
                .checked_sub(offset % granularity)
                .ok_or_else(|| io::Error::other("mapping alignment overflow"))?;
            let delta = usize::try_from(offset - aligned)
                .map_err(|_| io::Error::other("mapping offset is too large"))?;
            let view_len = delta
                .checked_add(len)
                .filter(|len| *len <= isize::MAX as usize)
                .ok_or_else(|| io::Error::other("mapping view is too large"))?;
            // SAFETY: strict has read access. The unnamed section is read-only;
            // zero maximum size uses the locked file's current size.
            let raw_mapping = unsafe {
                CreateFileMappingW(
                    strict.as_raw_handle() as isize,
                    std::ptr::null(),
                    PAGE_READONLY,
                    0,
                    0,
                    std::ptr::null(),
                )
            };
            if raw_mapping == 0 {
                return Err(io::Error::last_os_error());
            }
            let mapping = unsafe { OwnedHandle::from_raw_handle(raw_mapping as Handle) };
            let raw_view = unsafe {
                MapViewOfFile(
                    mapping.as_raw_handle() as isize,
                    FILE_MAP_READ,
                    (aligned >> 32) as u32,
                    aligned as u32,
                    view_len,
                )
            };
            let base = NonNull::new(raw_view.cast()).ok_or_else(io::Error::last_os_error)?;
            Ok(Self {
                base,
                delta,
                len,
                _mapping: mapping,
                _file: strict,
            })
        }

        pub(super) fn bytes(&self) -> &[u8] {
            // SAFETY: a successful view covers delta+len checked bytes in one
            // mapping, bounded by isize::MAX. PAGE_READONLY/FILE_MAP_READ cannot
            // mutate them, the strict File denies writers/truncation/writable
            // mappings, and nonlocal/unclassifiable files are never mapped.
            // All owners remain live until after the last borrowed slice.
            unsafe { std::slice::from_raw_parts(self.base.as_ptr().add(self.delta), self.len) }
        }
    }

    // SAFETY: the address is stable; the view is immutable under the OS share
    // lock, and File/OwnedHandle may cross threads. Drop requires exclusive
    // ownership and unmaps before releasing that lock.
    unsafe impl Send for Mapping {}
    unsafe impl Sync for Mapping {}

    impl Drop for Mapping {
        fn drop(&mut self) {
            // SAFETY: this is the exact base returned by MapViewOfFile, once.
            unsafe {
                UnmapViewOfFile(self.base.as_ptr().cast());
            }
        }
    }

    fn local_disk(file: &File) -> bool {
        // A server-side writer can bypass a client's file sharing restrictions;
        // never expose remote mapped bytes as an immutable Rust slice.
        let needed =
            unsafe { GetFinalPathNameByHandleW(file.as_raw_handle(), std::ptr::null_mut(), 0, 0) };
        if needed == 0 || needed > 32768 {
            return false;
        }
        let mut path = vec![0u16; needed as usize + 1];
        let written = unsafe {
            GetFinalPathNameByHandleW(
                file.as_raw_handle(),
                path.as_mut_ptr(),
                path.len() as u32,
                0,
            )
        };
        if written == 0 || written as usize >= path.len() {
            return false;
        }
        let Some(root) = local_dos_root(&path[..written as usize]) else {
            return false;
        };
        // DRIVE_REMOVABLE, DRIVE_FIXED, DRIVE_CDROM, DRIVE_RAMDISK. Reject
        // DRIVE_REMOTE, UNKNOWN, NO_ROOT_DIR and all future/unrecognized kinds.
        matches!(unsafe { GetDriveTypeW(root.as_ptr()) }, 2 | 3 | 5 | 6)
    }

    fn local_dos_root(path: &[u16]) -> Option<[u16; 4]> {
        let path = path.strip_prefix(&[92, 92, 63, 92]).unwrap_or(path);
        if path.len() < 3
            || !((65..=90).contains(&path[0]) || (97..=122).contains(&path[0]))
            || path[1] != 58
            || path[2] != 92
        {
            return None;
        }
        Some([path[0], 58, 92, 0])
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::fs::OpenOptions;
        use std::io::Write;
        use std::path::PathBuf;

        fn fixture(label: &str) -> (PathBuf, Vec<u8>, File) {
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "rsui-code-map-{}-{}-{label}.bin",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            let bytes: Vec<u8> = (0..150_123).map(|at| (at % 251) as u8).collect();
            std::fs::write(&path, &bytes).unwrap();
            let file = File::open(&path).unwrap();
            (path, bytes, file)
        }

        #[test]
        fn partial_mapping_matches_owned_bytes_and_holds_a_write_lock_until_drop() {
            let (path, bytes, file) = fixture("partial");
            let mapped = Mapping::new(&file, 65_543, 70_001).unwrap();
            assert_eq!(mapped.bytes(), &bytes[65_543..135_544]);
            assert!(OpenOptions::new().write(true).open(&path).is_err());
            assert!(
                OpenOptions::new()
                    .write(true)
                    .truncate(true)
                    .open(&path)
                    .is_err()
            );
            drop(mapped);
            let mut writer = OpenOptions::new().write(true).open(&path).unwrap();
            writer.write_all(b"after drop").unwrap();
            writer.set_len(10).unwrap();
            drop(writer);
            drop(file);
            std::fs::remove_file(path).unwrap();
        }

        #[test]
        fn mapping_uses_the_existing_handle_after_its_path_is_replaced() {
            let (path, bytes, file) = fixture("renamed");
            let renamed = path.with_extension("renamed");
            std::fs::rename(&path, &renamed).unwrap();
            std::fs::write(&path, b"a different object").unwrap();
            let mapped = Mapping::new(&file, 123, 321).unwrap();
            assert_eq!(mapped.bytes(), &bytes[123..444]);
            drop(mapped);
            drop(file);
            std::fs::remove_file(path).unwrap();
            std::fs::remove_file(renamed).unwrap();
        }

        #[test]
        fn existing_writer_forces_owned_fallback_and_invalid_ranges_are_rejected() {
            let (path, bytes, file) = fixture("fallback");
            let writer = OpenOptions::new().write(true).open(&path).unwrap();
            assert!(Mapping::new(&file, 0, 100).is_err());
            let owned = CodeBytes::read_mode(&file, 123, 321, true).unwrap();
            assert!(matches!(owned, CodeBytes::Owned(_)));
            assert_eq!(&*owned, &bytes[123..444]);
            assert!(Mapping::new(&file, u64::MAX, 2).is_err());
            assert!(CodeBytes::read_mode(&file, bytes.len() as u64, 1, false).is_err());
            assert!(CodeBytes::read_mode(&file, 0, u64::MAX, false).is_err());
            let empty = CodeBytes::read_mode(&file, 0, 0, true).unwrap();
            assert!(empty.is_empty());
            drop(writer);
            drop(file);
            std::fs::remove_file(path).unwrap();
        }

        #[test]
        fn writable_view_outliving_its_writer_handle_must_prevent_readonly_mapping() {
            let (path, bytes, file) = fixture("old-writable-view");
            let writer = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .unwrap();
            let raw = unsafe {
                CreateFileMappingW(
                    writer.as_raw_handle() as isize,
                    std::ptr::null(),
                    4,
                    0,
                    0,
                    std::ptr::null(),
                )
            };
            assert_ne!(raw, 0);
            let writable_section = unsafe { OwnedHandle::from_raw_handle(raw as Handle) };
            let writable_view = unsafe {
                MapViewOfFile(
                    writable_section.as_raw_handle() as isize,
                    2,
                    0,
                    0,
                    bytes.len(),
                )
            };
            assert!(!writable_view.is_null());
            drop(writer);
            // A view retains the section even after BOTH public handles close.
            // Test that case rather than relying on a section handle to keep a
            // sharing restriction that the remaining view might not preserve.
            drop(writable_section);
            // This is a soundness gate, not an optional performance assertion.
            // If the OS permits this, the mapped-storage implementation cannot
            // expose &[u8] and must be withdrawn or obtain a stronger proof.
            assert!(
                Mapping::new(&file, 0, 100).is_err(),
                "strict reopen permitted a live writable view"
            );
            unsafe {
                UnmapViewOfFile(writable_view);
            }
            drop(file);
            std::fs::remove_file(path).unwrap();
        }

        #[test]
        fn only_certifiable_dos_roots_are_eligible() {
            let wide = |text: &str| text.encode_utf16().collect::<Vec<_>>();
            assert_eq!(
                local_dos_root(&wide(r"\\?\C:\local\app.exe")),
                Some([67, 58, 92, 0])
            );
            assert!(local_dos_root(&wide(r"\\?\UNC\server\share\app.exe")).is_none());
            assert!(local_dos_root(&wide(r"\\?\Volume{unknown}\app.exe")).is_none());
            assert!(local_dos_root(&wide(r"C:relative")).is_none());
            fn send_sync<T: Send + Sync>() {}
            send_sync::<CodeBytes>();
        }
    }
}
