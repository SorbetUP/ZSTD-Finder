use std::fs::{self, File, Metadata};
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use tempfile::NamedTempFile;
use walkdir::WalkDir;
use xxhash_rust::xxh3::xxh3_64;

use crate::error::{Error, Result};
use crate::format::{
    ArchiveIndex, ChunkCodec, ChunkRef, Entry, EntryKind, Header, UnixTime,
    DEFAULT_CHUNK_SIZE, FORMAT_VERSION, HEADER_SIZE, MAX_CHUNK_SIZE, MIN_CHUNK_SIZE,
};
use crate::path::path_to_archive;

#[derive(Debug, Clone)]
pub struct PackOptions {
    pub chunk_size: u32,
    pub compression_level: i32,
    pub overwrite: bool,
}

impl Default for PackOptions {
    fn default() -> Self {
        Self {
            chunk_size: DEFAULT_CHUNK_SIZE,
            compression_level: 3,
            overwrite: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackSummary {
    pub entries: usize,
    pub files: usize,
    pub source_bytes: u64,
    pub archive_bytes: u64,
    pub compressed_chunks: u64,
    pub stored_chunks: u64,
}

pub fn pack_directory(
    source: impl AsRef<Path>,
    destination: impl AsRef<Path>,
    options: &PackOptions,
) -> Result<PackSummary> {
    validate_options(options)?;

    let source = fs::canonicalize(source.as_ref())?;
    if !source.is_dir() {
        return Err(Error::InvalidPath(format!(
            "source is not a directory: {}",
            source.display()
        )));
    }

    let destination = absolute_destination(destination.as_ref())?;
    if destination.exists() && !options.overwrite {
        return Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("destination already exists: {}", destination.display()),
        )));
    }

    let parent = destination.parent().ok_or_else(|| {
        Error::InvalidPath(format!("destination has no parent: {}", destination.display()))
    })?;
    fs::create_dir_all(parent)?;

    let mut paths = Vec::new();
    for item in WalkDir::new(&source).follow_links(false) {
        let item = item.map_err(|error| {
            Error::Io(
                error
                    .into_io_error()
                    .unwrap_or_else(|| std::io::Error::other("directory traversal failed")),
            )
        })?;
        if item.depth() != 0 {
            let path = item.into_path();
            let archive_path = path_to_archive(&source, &path)?;
            paths.push((archive_path, path));
        }
    }
    paths.sort_by(|a, b| a.0.cmp(&b.0));

    let mut temp = NamedTempFile::new_in(parent)?;
    temp.as_file_mut().write_all(&[0_u8; HEADER_SIZE])?;

    let mut entries = Vec::with_capacity(paths.len());
    let mut source_bytes = 0_u64;
    let mut compressed_chunks = 0_u64;
    let mut stored_chunks = 0_u64;
    let mut file_count = 0_usize;

    for (archive_path, path) in paths {
        let metadata = fs::symlink_metadata(&path)?;
        let common = EntryCommon::from_metadata(&metadata);

        if metadata.file_type().is_symlink() {
            let target = fs::read_link(&path)?;
            let target = target.to_str().ok_or_else(|| {
                Error::InvalidPath(format!("non-UTF-8 symlink target: {}", path.display()))
            })?;
            entries.push(Entry {
                path: archive_path,
                kind: EntryKind::Symlink,
                size: 0,
                unix_mode: common.unix_mode,
                modified: common.modified,
                chunks: Vec::new(),
                symlink_target: Some(target.to_owned()),
            });
            continue;
        }

        if metadata.is_dir() {
            entries.push(Entry {
                path: archive_path,
                kind: EntryKind::Directory,
                size: 0,
                unix_mode: common.unix_mode,
                modified: common.modified,
                chunks: Vec::new(),
                symlink_target: None,
            });
            continue;
        }

        if !metadata.is_file() {
            return Err(Error::InvalidPath(format!(
                "unsupported filesystem object: {}",
                path.display()
            )));
        }

        file_count += 1;
        source_bytes = source_bytes.saturating_add(metadata.len());
        let (chunks, compressed, stored) = write_file_chunks(
            temp.as_file_mut(),
            &path,
            options.chunk_size,
            options.compression_level,
        )?;
        compressed_chunks += compressed;
        stored_chunks += stored;

        entries.push(Entry {
            path: archive_path,
            kind: EntryKind::File,
            size: metadata.len(),
            unix_mode: common.unix_mode,
            modified: common.modified,
            chunks,
            symlink_target: None,
        });
    }

    let index = ArchiveIndex {
        version: FORMAT_VERSION,
        chunk_size: options.chunk_size,
        entries,
    };
    let index_raw = serde_json::to_vec(&index)?;
    let index_checksum = xxh3_64(&index_raw);
    let index_compressed = zstd::bulk::compress(&index_raw, options.compression_level)?;
    let index_offset = temp.as_file_mut().stream_position()?;
    temp.as_file_mut().write_all(&index_compressed)?;

    let header = Header {
        version: FORMAT_VERSION,
        flags: 0,
        chunk_size: options.chunk_size,
        index_offset,
        index_stored_len: index_compressed.len() as u64,
        index_raw_len: index_raw.len() as u64,
        index_checksum,
    };
    temp.as_file_mut().seek(SeekFrom::Start(0))?;
    temp.as_file_mut().write_all(&header.encode())?;
    temp.as_file_mut().flush()?;
    temp.as_file().sync_all()?;

    let archive_bytes = temp.as_file().metadata()?.len();
    temp.persist(&destination)
        .map_err(|error| Error::Io(error.error))?;

    Ok(PackSummary {
        entries: index.entries.len(),
        files: file_count,
        source_bytes,
        archive_bytes,
        compressed_chunks,
        stored_chunks,
    })
}

fn write_file_chunks(
    output: &mut File,
    path: &Path,
    chunk_size: u32,
    compression_level: i32,
) -> Result<(Vec<ChunkRef>, u64, u64)> {
    let file = File::open(path)?;
    let mut reader = BufReader::with_capacity(chunk_size as usize, file);
    let mut buffer = vec![0_u8; chunk_size as usize];
    let mut chunks = Vec::new();
    let mut compressed_count = 0_u64;
    let mut stored_count = 0_u64;

    loop {
        let mut filled = 0_usize;
        while filled < buffer.len() {
            let read = reader.read(&mut buffer[filled..])?;
            if read == 0 {
                break;
            }
            filled += read;
        }
        if filled == 0 {
            break;
        }

        let raw = &buffer[..filled];
        let compressed = zstd::bulk::compress(raw, compression_level)?;
        let (codec, data) = if compressed.len().saturating_add(8) < raw.len() {
            compressed_count += 1;
            (ChunkCodec::Zstd, compressed.as_slice())
        } else {
            stored_count += 1;
            (ChunkCodec::Stored, raw)
        };

        let offset = output.stream_position()?;
        output.write_all(data)?;
        chunks.push(ChunkRef {
            offset,
            stored_len: u32::try_from(data.len()).map_err(|_| {
                Error::InvalidFormat("chunk stored length does not fit in u32".into())
            })?,
            raw_len: u32::try_from(raw.len())
                .map_err(|_| Error::InvalidFormat("chunk raw length does not fit in u32".into()))?,
            codec,
            checksum: xxh3_64(raw),
        });

        if filled < buffer.len() {
            break;
        }
    }

    Ok((chunks, compressed_count, stored_count))
}

fn validate_options(options: &PackOptions) -> Result<()> {
    if !(MIN_CHUNK_SIZE..=MAX_CHUNK_SIZE).contains(&options.chunk_size) {
        return Err(Error::InvalidFormat(format!(
            "chunk size must be between {MIN_CHUNK_SIZE} and {MAX_CHUNK_SIZE} bytes"
        )));
    }
    if !(-7..=22).contains(&options.compression_level) {
        return Err(Error::InvalidFormat(
            "Zstd compression level must be between -7 and 22".into(),
        ));
    }
    Ok(())
}

fn absolute_destination(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_owned());
    }
    Ok(std::env::current_dir()?.join(path))
}

struct EntryCommon {
    unix_mode: u32,
    modified: Option<UnixTime>,
}

impl EntryCommon {
    fn from_metadata(metadata: &Metadata) -> Self {
        Self {
            unix_mode: unix_mode(metadata),
            modified: metadata.modified().ok().map(system_time_to_unix),
        }
    }
}

#[cfg(unix)]
fn unix_mode(metadata: &Metadata) -> u32 {
    use std::os::unix::fs::MetadataExt;
    metadata.mode()
}

#[cfg(not(unix))]
fn unix_mode(_metadata: &Metadata) -> u32 {
    0
}

fn system_time_to_unix(time: SystemTime) -> UnixTime {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => UnixTime {
            seconds: i64::try_from(duration.as_secs()).unwrap_or(i64::MAX),
            nanos: duration.subsec_nanos(),
        },
        Err(error) => {
            let duration = error.duration();
            if duration.subsec_nanos() == 0 {
                UnixTime {
                    seconds: -i64::try_from(duration.as_secs()).unwrap_or(i64::MAX),
                    nanos: 0,
                }
            } else {
                UnixTime {
                    seconds: -i64::try_from(duration.as_secs()).unwrap_or(i64::MAX) - 1,
                    nanos: 1_000_000_000 - duration.subsec_nanos(),
                }
            }
        }
    }
}
