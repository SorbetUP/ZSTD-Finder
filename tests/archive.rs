use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};

use tempfile::tempdir;
use zstd_finder::{pack_directory, Archive, ChunkCodec, EntryKind, PackOptions};

const TEST_CHUNK: u32 = 64 * 1024;

#[test]
fn round_trip_and_random_access_cross_chunk_boundaries() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir_all(source.join("nested/empty-dir")).unwrap();

    fs::write(source.join("hello.txt"), b"hello finder\n".repeat(20_000)).unwrap();
    fs::write(source.join("empty.bin"), []).unwrap();

    let binary: Vec<u8> = (0..(TEST_CHUNK as usize * 3 + 12_345))
        .map(|index| ((index * 73 + index / 7) & 0xff) as u8)
        .collect();
    fs::write(source.join("nested/binary.dat"), &binary).unwrap();

    let archive_path = temp.path().join("test.zstf");
    let summary = pack_directory(
        &source,
        &archive_path,
        &PackOptions {
            chunk_size: TEST_CHUNK,
            compression_level: 3,
            overwrite: false,
        },
    )
    .unwrap();
    assert_eq!(summary.files, 3);
    assert!(summary.compressed_chunks > 0);

    let archive = Archive::open(&archive_path).unwrap();
    assert_eq!(archive.chunk_size(), TEST_CHUNK);
    assert_eq!(archive.entry("nested").unwrap().kind, EntryKind::Directory);
    assert_eq!(
        archive.entry("/nested/binary.dat").unwrap().size,
        binary.len() as u64
    );

    let start = TEST_CHUNK as u64 - 211;
    let length = TEST_CHUNK as usize + 777;
    assert_eq!(
        archive.read_at("nested/binary.dat", start, length).unwrap(),
        binary[start as usize..start as usize + length]
    );
    assert_eq!(
        archive
            .read_at("nested/binary.dat", binary.len() as u64 + 10, 100)
            .unwrap(),
        Vec::<u8>::new()
    );

    let children: Vec<_> = archive
        .children("nested")
        .unwrap()
        .into_iter()
        .map(|entry| entry.path.as_str())
        .collect();
    assert_eq!(children, vec!["nested/binary.dat", "nested/empty-dir"]);

    archive.verify().unwrap();

    let extracted = temp.path().join("extracted");
    archive.extract_all(&extracted).unwrap();
    assert_eq!(
        fs::read(extracted.join("nested/binary.dat")).unwrap(),
        binary
    );
    assert_eq!(
        fs::read(extracted.join("hello.txt")).unwrap(),
        fs::read(source.join("hello.txt")).unwrap()
    );
    assert!(extracted.join("nested/empty-dir").is_dir());
}

#[test]
fn incompressible_data_is_stored_instead_of_expanded() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();

    let mut state = 0x1234_5678_9abc_def0_u64;
    let bytes: Vec<u8> = (0..TEST_CHUNK as usize)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect();
    fs::write(source.join("noise.bin"), &bytes).unwrap();

    let archive_path = temp.path().join("noise.zstf");
    pack_directory(
        &source,
        &archive_path,
        &PackOptions {
            chunk_size: TEST_CHUNK,
            compression_level: 3,
            overwrite: false,
        },
    )
    .unwrap();

    let archive = Archive::open(&archive_path).unwrap();
    let entry = archive.entry("noise.bin").unwrap();
    assert_eq!(entry.chunks.len(), 1);
    assert_eq!(entry.chunks[0].codec, ChunkCodec::Stored);
    assert_eq!(archive.read_at("noise.bin", 0, bytes.len()).unwrap(), bytes);
}

#[test]
fn corruption_is_detected_only_when_affected_chunk_is_read() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(
        source.join("data.bin"),
        b"A".repeat(TEST_CHUNK as usize * 2),
    )
    .unwrap();

    let archive_path = temp.path().join("corrupt.zstf");
    pack_directory(
        &source,
        &archive_path,
        &PackOptions {
            chunk_size: TEST_CHUNK,
            compression_level: 3,
            overwrite: false,
        },
    )
    .unwrap();

    let archive = Archive::open(&archive_path).unwrap();
    let chunk = archive.entry("data.bin").unwrap().chunks[0].clone();
    drop(archive);

    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&archive_path)
        .unwrap();
    file.seek(SeekFrom::Start(chunk.offset)).unwrap();
    let mut byte = [0_u8; 1];
    file.read_exact(&mut byte).unwrap();
    byte[0] ^= 0x5a;
    file.seek(SeekFrom::Start(chunk.offset)).unwrap();
    file.write_all(&byte).unwrap();
    file.flush().unwrap();

    let archive = Archive::open(&archive_path).unwrap();
    let second_chunk = archive.read_at("data.bin", TEST_CHUNK as u64, 32).unwrap();
    assert_eq!(second_chunk, vec![b'A'; 32]);
    assert!(archive.read_at("data.bin", 0, 1).is_err());
}

#[test]
fn truncated_archive_is_rejected_before_payload_reads() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("data.bin"), b"abcdef").unwrap();

    let archive_path = temp.path().join("truncated.zstf");
    pack_directory(&source, &archive_path, &PackOptions::default()).unwrap();
    let len = fs::metadata(&archive_path).unwrap().len();
    OpenOptions::new()
        .write(true)
        .open(&archive_path)
        .unwrap()
        .set_len(len - 1)
        .unwrap();

    assert!(Archive::open(&archive_path).is_err());
}

#[cfg(unix)]
#[test]
fn symlinks_round_trip_without_following_them() {
    use std::os::unix::fs::symlink;

    let temp = tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("target.txt"), b"target").unwrap();
    symlink("target.txt", source.join("link.txt")).unwrap();

    let archive_path = temp.path().join("links.zstf");
    pack_directory(&source, &archive_path, &PackOptions::default()).unwrap();
    let archive = Archive::open(&archive_path).unwrap();
    assert_eq!(archive.entry("link.txt").unwrap().kind, EntryKind::Symlink);
    assert_eq!(
        archive.entry("link.txt").unwrap().symlink_target.as_deref(),
        Some("target.txt")
    );

    let extracted = temp.path().join("out");
    archive.extract_all(&extracted).unwrap();
    assert_eq!(
        fs::read_link(extracted.join("link.txt")).unwrap(),
        std::path::PathBuf::from("target.txt")
    );
}
