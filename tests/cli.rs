use std::fs;
use std::process::Command;

use tempfile::tempdir;

#[test]
fn cli_pack_verify_list_read_and_extract() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir_all(source.join("album")).unwrap();
    let photo = b"fake-photo-payload".repeat(20_000);
    fs::write(source.join("album/photo.raw"), &photo).unwrap();

    let archive = temp.path().join("photos.zstf");
    let binary = env!("CARGO_BIN_EXE_zstd-finder");

    let status = Command::new(binary)
        .args(["pack", source.to_str().unwrap(), archive.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(status.success());

    let verify = Command::new(binary)
        .args(["verify", archive.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(verify.status.success());
    assert!(String::from_utf8_lossy(&verify.stdout).contains("OK:"));

    let list = Command::new(binary)
        .args(["list", archive.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(list.status.success());
    assert!(String::from_utf8_lossy(&list.stdout).contains("album/photo.raw"));

    let range = Command::new(binary)
        .args([
            "read",
            archive.to_str().unwrap(),
            "album/photo.raw",
            "12345",
            "54321",
        ])
        .output()
        .unwrap();
    assert!(range.status.success());
    assert_eq!(range.stdout, photo[12_345..12_345 + 54_321]);

    let destination = temp.path().join("out");
    let extract = Command::new(binary)
        .args([
            "extract",
            archive.to_str().unwrap(),
            destination.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(extract.success());
    assert_eq!(
        fs::read(destination.join("album/photo.raw")).unwrap(),
        photo
    );
}
