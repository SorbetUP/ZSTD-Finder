use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::ptr;

use crate::{Archive, EntryKind, FORMAT_VERSION};

pub const ZSTF_OK: c_int = 0;
pub const ZSTF_ERR_INVALID_ARGUMENT: c_int = -1;
pub const ZSTF_ERR_BUFFER_TOO_SMALL: c_int = -2;
pub const ZSTF_ERR_ARCHIVE: c_int = -3;
pub const ZSTF_ERR_PANIC: c_int = -127;

pub const ZSTF_KIND_FILE: u32 = 1;
pub const ZSTF_KIND_DIRECTORY: u32 = 2;
pub const ZSTF_KIND_SYMLINK: u32 = 3;

thread_local! {
    static LAST_ERROR: RefCell<CString> = RefCell::new(CString::new("").expect("empty CString"));
}

pub struct ZstfArchiveHandle {
    archive: Archive,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ZstfEntryMetadata {
    pub kind: u32,
    pub unix_mode: u32,
    pub size: u64,
    pub modified_seconds: i64,
    pub modified_nanos: u32,
    pub has_modified: u32,
}

#[no_mangle]
pub extern "C" fn zstf_format_version() -> u16 {
    FORMAT_VERSION
}

/// Opens a `.zstf` archive and returns an opaque read-only handle.
///
/// # Safety
/// `path` must point to a valid NUL-terminated UTF-8 C string and `out_handle`
/// must point to writable memory for one handle pointer.
#[no_mangle]
pub unsafe extern "C" fn zstf_archive_open(
    path: *const c_char,
    out_handle: *mut *mut ZstfArchiveHandle,
) -> c_int {
    ffi_call(|| {
        if path.is_null() || out_handle.is_null() {
            return fail(ZSTF_ERR_INVALID_ARGUMENT, "null pointer passed to zstf_archive_open");
        }
        *out_handle = ptr::null_mut();
        let path = match CStr::from_ptr(path).to_str() {
            Ok(path) => path,
            Err(error) => {
                return fail(
                    ZSTF_ERR_INVALID_ARGUMENT,
                    format!("archive path is not UTF-8: {error}"),
                )
            }
        };
        let archive = match Archive::open(Path::new(path)) {
            Ok(archive) => archive,
            Err(error) => return fail(ZSTF_ERR_ARCHIVE, error.to_string()),
        };
        *out_handle = Box::into_raw(Box::new(ZstfArchiveHandle { archive }));
        clear_error();
        ZSTF_OK
    })
}

/// Closes a handle returned by [`zstf_archive_open`].
///
/// # Safety
/// `handle` must either be null or a live handle returned by
/// [`zstf_archive_open`] that has not already been closed.
#[no_mangle]
pub unsafe extern "C" fn zstf_archive_close(handle: *mut ZstfArchiveHandle) {
    if handle.is_null() {
        return;
    }
    let _ = catch_unwind(AssertUnwindSafe(|| drop(Box::from_raw(handle))));
}

/// Returns the number of indexed entries in an archive.
///
/// # Safety
/// `handle` must be a live archive handle and `out_count` must point to writable
/// memory for one `usize`.
#[no_mangle]
pub unsafe extern "C" fn zstf_archive_entry_count(
    handle: *const ZstfArchiveHandle,
    out_count: *mut usize,
) -> c_int {
    ffi_call(|| {
        let handle = match handle.as_ref() {
            Some(handle) => handle,
            None => return fail(ZSTF_ERR_INVALID_ARGUMENT, "null archive handle"),
        };
        if out_count.is_null() {
            return fail(ZSTF_ERR_INVALID_ARGUMENT, "null out_count pointer");
        }
        *out_count = handle.archive.entries().len();
        clear_error();
        ZSTF_OK
    })
}

/// Copies one entry's archive-relative UTF-8 path into `buffer`.
///
/// Call once with a null buffer and zero capacity to discover the required
/// byte length through `out_len`, then call again with a sufficiently large
/// buffer. The copied bytes are not NUL-terminated.
///
/// # Safety
/// `handle` must be live. `out_len` must be writable. If `buffer` is non-null,
/// it must reference at least `capacity` writable bytes.
#[no_mangle]
pub unsafe extern "C" fn zstf_archive_entry_path(
    handle: *const ZstfArchiveHandle,
    index: usize,
    buffer: *mut u8,
    capacity: usize,
    out_len: *mut usize,
) -> c_int {
    ffi_call(|| {
        let handle = match handle.as_ref() {
            Some(handle) => handle,
            None => return fail(ZSTF_ERR_INVALID_ARGUMENT, "null archive handle"),
        };
        let entry = match handle.archive.entries().get(index) {
            Some(entry) => entry,
            None => {
                return fail(
                    ZSTF_ERR_INVALID_ARGUMENT,
                    format!("entry index {index} is out of range"),
                )
            }
        };
        copy_bytes(entry.path.as_bytes(), buffer, capacity, out_len)
    })
}

/// Returns metadata for one indexed entry.
///
/// # Safety
/// `handle` must be live and `out_metadata` must point to writable memory for
/// one [`ZstfEntryMetadata`].
#[no_mangle]
pub unsafe extern "C" fn zstf_archive_entry_metadata(
    handle: *const ZstfArchiveHandle,
    index: usize,
    out_metadata: *mut ZstfEntryMetadata,
) -> c_int {
    ffi_call(|| {
        let handle = match handle.as_ref() {
            Some(handle) => handle,
            None => return fail(ZSTF_ERR_INVALID_ARGUMENT, "null archive handle"),
        };
        if out_metadata.is_null() {
            return fail(ZSTF_ERR_INVALID_ARGUMENT, "null metadata pointer");
        }
        let entry = match handle.archive.entries().get(index) {
            Some(entry) => entry,
            None => {
                return fail(
                    ZSTF_ERR_INVALID_ARGUMENT,
                    format!("entry index {index} is out of range"),
                )
            }
        };
        let kind = match entry.kind {
            EntryKind::File => ZSTF_KIND_FILE,
            EntryKind::Directory => ZSTF_KIND_DIRECTORY,
            EntryKind::Symlink => ZSTF_KIND_SYMLINK,
        };
        let (modified_seconds, modified_nanos, has_modified) = entry
            .modified
            .map(|time| (time.seconds, time.nanos, 1))
            .unwrap_or((0, 0, 0));
        *out_metadata = ZstfEntryMetadata {
            kind,
            unix_mode: entry.unix_mode,
            size: entry.size,
            modified_seconds,
            modified_nanos,
            has_modified,
        };
        clear_error();
        ZSTF_OK
    })
}

/// Copies a symbolic link target using the same two-call convention as
/// [`zstf_archive_entry_path`].
///
/// # Safety
/// The same pointer requirements as [`zstf_archive_entry_path`] apply.
#[no_mangle]
pub unsafe extern "C" fn zstf_archive_entry_symlink_target(
    handle: *const ZstfArchiveHandle,
    index: usize,
    buffer: *mut u8,
    capacity: usize,
    out_len: *mut usize,
) -> c_int {
    ffi_call(|| {
        let handle = match handle.as_ref() {
            Some(handle) => handle,
            None => return fail(ZSTF_ERR_INVALID_ARGUMENT, "null archive handle"),
        };
        let entry = match handle.archive.entries().get(index) {
            Some(entry) => entry,
            None => {
                return fail(
                    ZSTF_ERR_INVALID_ARGUMENT,
                    format!("entry index {index} is out of range"),
                )
            }
        };
        let target = entry.symlink_target.as_deref().unwrap_or("");
        copy_bytes(target.as_bytes(), buffer, capacity, out_len)
    })
}

/// Reads at most `length` logical bytes from one archived regular file without
/// decoding unrelated chunks.
///
/// # Safety
/// `handle` must be live and `path` must be a valid NUL-terminated UTF-8 C
/// string. `out_read` must be writable. For a non-zero `length`, `buffer` must
/// reference at least `length` writable bytes.
#[no_mangle]
pub unsafe extern "C" fn zstf_archive_read(
    handle: *const ZstfArchiveHandle,
    path: *const c_char,
    offset: u64,
    buffer: *mut u8,
    length: usize,
    out_read: *mut usize,
) -> c_int {
    ffi_call(|| {
        let handle = match handle.as_ref() {
            Some(handle) => handle,
            None => return fail(ZSTF_ERR_INVALID_ARGUMENT, "null archive handle"),
        };
        if path.is_null() || out_read.is_null() || (length != 0 && buffer.is_null()) {
            return fail(ZSTF_ERR_INVALID_ARGUMENT, "invalid read pointer argument");
        }
        *out_read = 0;
        let path = match CStr::from_ptr(path).to_str() {
            Ok(path) => path,
            Err(error) => {
                return fail(
                    ZSTF_ERR_INVALID_ARGUMENT,
                    format!("entry path is not UTF-8: {error}"),
                )
            }
        };
        let data = match handle.archive.read_at(path, offset, length) {
            Ok(data) => data,
            Err(error) => return fail(ZSTF_ERR_ARCHIVE, error.to_string()),
        };
        if !data.is_empty() {
            ptr::copy_nonoverlapping(data.as_ptr(), buffer, data.len());
        }
        *out_read = data.len();
        clear_error();
        ZSTF_OK
    })
}

/// Copies the calling thread's most recent FFI error as a NUL-terminated UTF-8
/// string. Returns the complete byte length excluding the terminator.
///
/// # Safety
/// If `buffer` is non-null it must reference at least `capacity` writable bytes.
#[no_mangle]
pub unsafe extern "C" fn zstf_last_error(buffer: *mut c_char, capacity: usize) -> usize {
    LAST_ERROR.with(|slot| {
        let error = slot.borrow();
        let bytes = error.as_bytes();
        if buffer.is_null() || capacity == 0 {
            return bytes.len();
        }
        let copy_len = bytes.len().min(capacity.saturating_sub(1));
        ptr::copy_nonoverlapping(bytes.as_ptr().cast::<c_char>(), buffer, copy_len);
        *buffer.add(copy_len) = 0;
        bytes.len()
    })
}

unsafe fn copy_bytes(
    bytes: &[u8],
    buffer: *mut u8,
    capacity: usize,
    out_len: *mut usize,
) -> c_int {
    if out_len.is_null() {
        return fail(ZSTF_ERR_INVALID_ARGUMENT, "null output length pointer");
    }
    *out_len = bytes.len();
    if bytes.is_empty() || (buffer.is_null() && capacity == 0) {
        clear_error();
        return ZSTF_OK;
    }
    if buffer.is_null() {
        return fail(ZSTF_ERR_INVALID_ARGUMENT, "null output buffer");
    }
    if capacity < bytes.len() {
        return fail(
            ZSTF_ERR_BUFFER_TOO_SMALL,
            format!("buffer needs {} bytes, got {capacity}", bytes.len()),
        );
    }
    ptr::copy_nonoverlapping(bytes.as_ptr(), buffer, bytes.len());
    clear_error();
    ZSTF_OK
}

fn ffi_call(operation: impl FnOnce() -> c_int) -> c_int {
    match catch_unwind(AssertUnwindSafe(operation)) {
        Ok(code) => code,
        Err(_) => fail(ZSTF_ERR_PANIC, "panic crossed the native FFI boundary"),
    }
}

fn fail(code: c_int, message: impl Into<String>) -> c_int {
    set_error(message.into());
    code
}

fn clear_error() {
    set_error(String::new());
}

fn set_error(message: String) {
    let sanitized = message.replace('\0', "\\0");
    let value = CString::new(sanitized).unwrap_or_else(|_| CString::new("FFI error").unwrap());
    LAST_ERROR.with(|slot| *slot.borrow_mut() = value);
}
