use std::ffi::{CStr, CString};
use std::fs;
use std::ptr;

use tempfile::tempdir;
use zstd_finder::ffi::{
    zstf_archive_close, zstf_archive_entry_count, zstf_archive_entry_metadata,
    zstf_archive_entry_path, zstf_archive_open, zstf_archive_read, zstf_last_error,
    ZstfArchiveHandle, ZstfEntryMetadata, ZSTF_KIND_DIRECTORY, ZSTF_KIND_FILE, ZSTF_OK,
};
use zstd_finder::{pack_directory, PackOptions};

#[test]
fn ffi_exposes_index_and_random_reads() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir_all(source.join("album")).unwrap();
    let payload = b"finder-range-read".repeat(20_000);
    fs::write(source.join("album/photo.raw"), &payload).unwrap();

    let archive_path = temp.path().join("photos.zstf");
    pack_directory(&source, &archive_path, &PackOptions::default()).unwrap();

    let path = CString::new(archive_path.to_str().unwrap()).unwrap();
    let mut handle: *mut ZstfArchiveHandle = ptr::null_mut();
    assert_eq!(unsafe { zstf_archive_open(path.as_ptr(), &mut handle) }, ZSTF_OK);
    assert!(!handle.is_null());

    let mut count = 0_usize;
    assert_eq!(unsafe { zstf_archive_entry_count(handle, &mut count) }, ZSTF_OK);
    assert_eq!(count, 2);

    let first_path = entry_path(handle, 0);
    let second_path = entry_path(handle, 1);
    assert_eq!(first_path, "album");
    assert_eq!(second_path, "album/photo.raw");

    let mut directory_metadata = ZstfEntryMetadata::default();
    assert_eq!(
        unsafe { zstf_archive_entry_metadata(handle, 0, &mut directory_metadata) },
        ZSTF_OK
    );
    assert_eq!(directory_metadata.kind, ZSTF_KIND_DIRECTORY);

    let mut file_metadata = ZstfEntryMetadata::default();
    assert_eq!(
        unsafe { zstf_archive_entry_metadata(handle, 1, &mut file_metadata) },
        ZSTF_OK
    );
    assert_eq!(file_metadata.kind, ZSTF_KIND_FILE);
    assert_eq!(file_metadata.size, payload.len() as u64);

    let archived_path = CString::new("album/photo.raw").unwrap();
    let mut output = vec![0_u8; 77_777];
    let mut read = 0_usize;
    assert_eq!(
        unsafe {
            zstf_archive_read(
                handle,
                archived_path.as_ptr(),
                12_345,
                output.as_mut_ptr(),
                output.len(),
                &mut read,
            )
        },
        ZSTF_OK
    );
    output.truncate(read);
    assert_eq!(output, payload[12_345..12_345 + 77_777]);

    unsafe { zstf_archive_close(handle) };
}

#[test]
fn ffi_reports_errors_without_unwinding() {
    let missing = CString::new("/definitely/missing/archive.zstf").unwrap();
    let mut handle: *mut ZstfArchiveHandle = ptr::null_mut();
    assert_ne!(unsafe { zstf_archive_open(missing.as_ptr(), &mut handle) }, ZSTF_OK);
    assert!(handle.is_null());

    let needed = unsafe { zstf_last_error(ptr::null_mut(), 0) };
    assert!(needed > 0);
    let mut error = vec![0_i8; needed + 1];
    let reported = unsafe { zstf_last_error(error.as_mut_ptr(), error.len()) };
    assert_eq!(reported, needed);
    let message = unsafe { CStr::from_ptr(error.as_ptr()) }
        .to_str()
        .unwrap();
    assert!(!message.is_empty());
}

fn entry_path(handle: *const ZstfArchiveHandle, index: usize) -> String {
    let mut len = 0_usize;
    assert_eq!(
        unsafe { zstf_archive_entry_path(handle, index, ptr::null_mut(), 0, &mut len) },
        ZSTF_OK
    );
    let mut buffer = vec![0_u8; len];
    assert_eq!(
        unsafe {
            zstf_archive_entry_path(handle, index, buffer.as_mut_ptr(), buffer.len(), &mut len)
        },
        ZSTF_OK
    );
    String::from_utf8(buffer).unwrap()
}
