//! A whole file as read-only memory, for files that are large and little read, such as fonts: only the pages actually
//! touched take up any memory, and other programs using the same file share them.
//!
//! On Windows rsui-text maps its own (it denies everyone else writing to the file while it's open, which Unix can't).
//! On Unix a file someone shortens while it's mapped would fault where it's read past its new end. Fonts and packed
//! programs aren't changed in place by anything that installs them (packages replace files), so that's accepted, as
//! every other program that maps its fonts does.

/// A read-only view of a whole file, until it's dropped.
pub struct Mapping {
    address: *const u8,
    len: usize,
}

// SAFETY: the view is read-only memory that nothing writes, and it's only given back on drop.
unsafe impl Send for Mapping {}
// SAFETY: as above.
unsafe impl Sync for Mapping {}

impl Mapping {
    /// The bytes of the file.
    pub fn bytes(&self) -> &[u8] {
        // SAFETY: the view covers `len` readable bytes until drop.
        unsafe { std::slice::from_raw_parts(self.address, self.len) }
    }
}

#[cfg(unix)]
impl Mapping {
    /// Maps the whole of the file at `path`. An empty file is an error, as there's nothing to map.
    pub fn open(path: &std::path::Path) -> std::io::Result<Self> {
        use std::os::fd::AsRawFd;
        let file = std::fs::File::open(path)?;
        let len = usize::try_from(file.metadata()?.len())
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "the file is too big to map"))?;
        if len == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "the file is empty"));
        }
        // SAFETY: a new read-only private mapping of a file that's open; the mapping keeps what it needs of the file
        // once that's closed.
        let address = unsafe {
            libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ, libc::MAP_PRIVATE, file.as_raw_fd(), 0)
        };
        if address == libc::MAP_FAILED {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Self { address: address.cast_const().cast(), len })
    }
}

#[cfg(unix)]
impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: this came from `open`, and is given back only here.
        unsafe { libc::munmap(self.address.cast_mut().cast(), self.len) };
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn a_mapped_file_reads_as_its_contents() {
        let path = std::env::temp_dir().join(format!("rsui-os-map-{}", std::process::id()));
        std::fs::write(&path, b"some bytes of a file").unwrap();
        let mapping = Mapping::open(&path).unwrap();
        assert_eq!(mapping.bytes(), b"some bytes of a file");
        drop(mapping);
        std::fs::remove_file(&path).unwrap();
        assert!(Mapping::open(&path).is_err());
    }
}
