//! Where a font's bytes live.

use std::io;
use std::path::Path;

/// A font file's contents. Fonts are large and little of them is ever read, so on
/// Windows a file is mapped into memory: only the pages actually touched take up any,
/// and other programs using the same file share them.
pub(crate) enum FontData {
    Owned(Box<[u8]>),
    Static(&'static [u8]),
    #[cfg(windows)]
    Mapped(windows::Mapping),
    #[cfg(unix)]
    Mapped(rsui_os::map::Mapping),
}

impl FontData {
    pub(crate) fn bytes(&self) -> &[u8] {
        match self {
            Self::Owned(bytes) => bytes,
            Self::Static(bytes) => bytes,
            #[cfg(any(windows, unix))]
            Self::Mapped(mapping) => mapping.bytes(),
        }
    }

    /// Whether this is a file on the disk, rather than only in memory.
    pub(crate) fn is_on_the_disk(&self) -> bool {
        match self {
            #[cfg(any(windows, unix))]
            Self::Mapped(_) => true,
            #[cfg(not(any(windows, unix)))]
            Self::Owned(_) => true,
            _ => false,
        }
    }

    pub(crate) fn open(path: &Path) -> io::Result<Self> {
        // What's inside a packed app isn't on the disk to map.
        if let Some(bytes) = crate::files::read_by_reader(path) {
            return Ok(Self::Owned(bytes.into()));
        }
        #[cfg(windows)]
        {
            windows::Mapping::open(path).map(Self::Mapped)
        }
        #[cfg(unix)]
        {
            rsui_os::map::Mapping::open(path).map(Self::Mapped)
        }
        #[cfg(not(any(windows, unix)))]
        {
            std::fs::read(path).map(|bytes| Self::Owned(bytes.into()))
        }
    }
}

#[cfg(windows)]
mod windows {
    use std::ffi::c_void;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    type Handle = *mut c_void;

    const GENERIC_READ: u32 = 0x8000_0000;
    const FILE_SHARE_READ: u32 = 1;
    const OPEN_EXISTING: u32 = 3;
    const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
    const PAGE_READONLY: u32 = 2;
    const FILE_MAP_READ: u32 = 4;
    const INVALID_HANDLE: Handle = -1isize as Handle;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateFileW(
            name: *const u16,
            access: u32,
            share: u32,
            security: *mut c_void,
            disposition: u32,
            flags: u32,
            template: Handle,
        ) -> Handle;
        fn GetFileSizeEx(file: Handle, size: *mut i64) -> i32;
        fn CreateFileMappingW(
            file: Handle,
            security: *mut c_void,
            protect: u32,
            size_high: u32,
            size_low: u32,
            name: *const u16,
        ) -> Handle;
        fn MapViewOfFile(
            mapping: Handle,
            access: u32,
            offset_high: u32,
            offset_low: u32,
            size: usize,
        ) -> *mut c_void;
        fn UnmapViewOfFile(address: *const c_void) -> i32;
        fn CloseHandle(handle: Handle) -> i32;
    }

    /// A read-only view of a whole file.
    pub(crate) struct Mapping {
        address: *const u8,
        len: usize,
        file: Handle,
    }

    impl Mapping {
        pub(crate) fn open(path: &Path) -> io::Result<Self> {
            let name: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
            // SAFETY: `name` is a null-terminated wide string, and everything returned is
            // checked. Sharing only for reading means nobody can change the file for as
            // long as `file` stays open, which is what makes the view safe to read as a
            // slice. It stays open until `Drop`.
            unsafe {
                let file = CreateFileW(
                    name.as_ptr(),
                    GENERIC_READ,
                    FILE_SHARE_READ,
                    std::ptr::null_mut(),
                    OPEN_EXISTING,
                    FILE_ATTRIBUTE_NORMAL,
                    std::ptr::null_mut(),
                );
                if file == INVALID_HANDLE {
                    return Err(io::Error::last_os_error());
                }
                let mut size = 0i64;
                if GetFileSizeEx(file, &mut size) == 0 {
                    let error = io::Error::last_os_error();
                    CloseHandle(file);
                    return Err(error);
                }
                // An empty file can't be mapped, and isn't a font either.
                if size <= 0 {
                    CloseHandle(file);
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "the file is empty",
                    ));
                }
                let mapping = CreateFileMappingW(
                    file,
                    std::ptr::null_mut(),
                    PAGE_READONLY,
                    0,
                    0,
                    std::ptr::null(),
                );
                if mapping.is_null() {
                    let error = io::Error::last_os_error();
                    CloseHandle(file);
                    return Err(error);
                }
                let address = MapViewOfFile(mapping, FILE_MAP_READ, 0, 0, 0);
                // The view keeps the mapping alive, so this handle isn't needed any more.
                CloseHandle(mapping);
                if address.is_null() {
                    let error = io::Error::last_os_error();
                    CloseHandle(file);
                    return Err(error);
                }
                Ok(Self {
                    address: address.cast_const().cast(),
                    len: size as usize,
                    file,
                })
            }
        }

        pub(crate) fn bytes(&self) -> &[u8] {
            // SAFETY: the view covers `len` readable bytes until `Drop`, and they can't
            // change, since `file` stays open and denies write access to everyone.
            unsafe { std::slice::from_raw_parts(self.address, self.len) }
        }
    }

    impl Drop for Mapping {
        fn drop(&mut self) {
            // SAFETY: these came from `open`, and are released only here.
            unsafe {
                UnmapViewOfFile(self.address.cast());
                CloseHandle(self.file);
            }
        }
    }
}
