use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use filetime::{set_file_mtime, FileTime};
use xxhash_rust::xxh3::xxh3_64;

use crate::error::{Error, Result};
use crate::format::{
    ArchiveIndex, ChunkCodec, ChunkRef, Entry, EntryKind, Header, FORMAT_VERSION, HEADER_SIZE,
    MAX_CHUNK_SIZE, MAX_INDEX_SIZE, MIN_CHUNK_SIZE,
};
use crate::path::{normalize_lookup_path, parent_path, validate_archive_path};

pub struct Archive {
    file: Mutex<File>,
    header: Header,
    index: ArchiveIndex,
    lookup: HashMap<String, usize>,
    children: HashMap<String, Vec<usize>>,
}

impl Archive {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let mut file = File::open(path)?;
        let file_len = file.metadata()?.len();
        if file_len < HEADER_SIZE as u64 {
            return Err(Error::InvalidFormat(
                "file is smaller than the header".into(),
            ));
        }

        let mut header_bytes = [0_u8; HEADER_SIZE];
        file.read_exact(&mut header_bytes)?;
        let header = Header::decode(&header_bytes)
            .map_err(|message| Error::InvalidFormat(message.into()))?;
        validate_header(header, file_len)?;

        file.seek(SeekFrom::Start(header.index_offset))?;
        let stored_len = usize::try_from(header.index_stored_len)
            .map_err(|_| Error::InvalidFormat("index is too large for this platform".into()))?;
        let mut compressed_index = vec![0_u8; stored_len];
        file.read_exact(&mut compressed_index)?;

        let raw_len = usize::try_from(header.index_raw_len)
            .map_err(|_| Error::InvalidFormat("index is too large for this platform".into()))?;
        let index_bytes = zstd::bulk::decompress(&compressed_index, raw_len)
            .map_err(|error| Error::InvalidFormat(format!("cannot decompress index: {error}")))?;
        if index_bytes.len() != raw_len {
            return Err(Error::InvalidFormat(format!(
                "index length mismatch: expected {raw_len}, got {}",
                index_bytes.len()
            )));
        }
        if xxh3_64(&index_bytes) != header.index_checksum {
            return Err(Error::InvalidFormat("index checksum mismatch".into()));
        }

        let index: ArchiveIndex = serde_json::from_slice(&index_bytes)?;
        let (lookup, children) = validate_index(&index, header)?;

        Ok(Self {
            file: Mutex::new(file),
            header,
            index,
            lookup,
            children,
        })
    }

    pub fn chunk_size(&self) -> u32 {
        self.header.chunk_size
    }

    pub fn entries(&self) -> &[Entry] {
        &self.index.entries
    }

    pub fn entry(&self, path: &str) -> Result<&Entry> {
        let path = normalize_lookup_path(path)?;
        let index = self
            .lookup
            .get(&path)
            .copied()
            .ok_or_else(|| Error::NotFound(path.clone()))?;
        Ok(&self.index.entries[index])
    }

    pub fn children(&self, path: &str) -> Result<Vec<&Entry>> {
        let path = normalize_lookup_path(path)?;
        if !path.is_empty() {
            let entry = self.entry(&path)?;
            if entry.kind != EntryKind::Directory {
                return Err(Error::InvalidPath(format!("not a directory: {path}")));
            }
        }

        Ok(self
            .children
            .get(&path)
            .into_iter()
            .flatten()
            .map(|index| &self.index.entries[*index])
            .collect())
    }

    pub fn read_at(&self, path: &str, offset: u64, length: usize) -> Result<Vec<u8>> {
        let entry = self.entry(path)?;
        if entry.kind != EntryKind::File {
            return Err(Error::NotAFile(entry.path.clone()));
        }
        if length == 0 || offset >= entry.size {
            return Ok(Vec::new());
        }

        let available = entry.size - offset;
        let wanted = usize::try_from(available.min(length as u64)).unwrap_or(length);
        let chunk_size = self.header.chunk_size as u64;
        let first_chunk = usize::try_from(offset / chunk_size)
            .map_err(|_| Error::InvalidFormat("chunk index overflow".into()))?;
        let last_byte = offset + wanted as u64 - 1;
        let last_chunk = usize::try_from(last_byte / chunk_size)
            .map_err(|_| Error::InvalidFormat("chunk index overflow".into()))?;

        let mut result = Vec::with_capacity(wanted);
        for chunk_index in first_chunk..=last_chunk {
            let chunk = entry.chunks.get(chunk_index).ok_or_else(|| {
                Error::InvalidFormat(format!("missing chunk {chunk_index} for {}", entry.path))
            })?;
            let decoded = self.read_chunk(&entry.path, chunk_index, chunk)?;
            let chunk_start = chunk_index as u64 * chunk_size;
            let copy_start = offset.saturating_sub(chunk_start) as usize;
            let request_end = offset + wanted as u64;
            let copy_end = decoded
                .len()
                .min(request_end.saturating_sub(chunk_start) as usize);
            if copy_start < copy_end {
                result.extend_from_slice(&decoded[copy_start..copy_end]);
            }
        }
        result.truncate(wanted);
        Ok(result)
    }

    pub fn verify(&self) -> Result<VerifySummary> {
        let mut files = 0_usize;
        let mut chunks = 0_u64;
        let mut bytes = 0_u64;
        for entry in &self.index.entries {
            if entry.kind != EntryKind::File {
                continue;
            }
            files += 1;
            bytes = bytes.saturating_add(entry.size);
            for (chunk_index, chunk) in entry.chunks.iter().enumerate() {
                self.read_chunk(&entry.path, chunk_index, chunk)?;
                chunks += 1;
            }
        }
        Ok(VerifySummary {
            entries: self.index.entries.len(),
            files,
            chunks,
            bytes,
        })
    }

    pub fn extract_all(&self, destination: impl AsRef<Path>) -> Result<()> {
        let destination = destination.as_ref();
        fs::create_dir_all(destination)?;

        for entry in &self.index.entries {
            if entry.kind == EntryKind::Directory {
                fs::create_dir_all(destination.join(path_from_archive(&entry.path)))?;
            }
        }
        for entry in &self.index.entries {
            match entry.kind {
                EntryKind::Directory => {}
                EntryKind::File => self.extract_file(entry, destination)?,
                EntryKind::Symlink => self.extract_symlink(entry, destination)?,
            }
        }
        Ok(())
    }

    pub fn extract_path(&self, path: &str, destination: impl AsRef<Path>) -> Result<()> {
        let entry = self.entry(path)?;
        let destination = destination.as_ref();
        fs::create_dir_all(destination)?;
        match entry.kind {
            EntryKind::Directory => {
                let prefix = format!("{}/", entry.path);
                fs::create_dir_all(destination.join(path_from_archive(&entry.path)))?;
                for child in &self.index.entries {
                    if child.path.starts_with(&prefix) {
                        match child.kind {
                            EntryKind::Directory => fs::create_dir_all(
                                destination.join(path_from_archive(&child.path)),
                            )?,
                            EntryKind::File => self.extract_file(child, destination)?,
                            EntryKind::Symlink => self.extract_symlink(child, destination)?,
                        }
                    }
                }
                Ok(())
            }
            EntryKind::File => self.extract_file(entry, destination),
            EntryKind::Symlink => self.extract_symlink(entry, destination),
        }
    }

    fn read_chunk(&self, path: &str, chunk_index: usize, chunk: &ChunkRef) -> Result<Vec<u8>> {
        let mut stored = vec![0_u8; chunk.stored_len as usize];
        {
            let mut file = self
                .file
                .lock()
                .map_err(|_| Error::Io(std::io::Error::other("archive file lock poisoned")))?;
            file.seek(SeekFrom::Start(chunk.offset))?;
            file.read_exact(&mut stored)?;
        }

        let decoded =
            match chunk.codec {
                ChunkCodec::Stored => stored,
                ChunkCodec::Zstd => zstd::bulk::decompress(&stored, chunk.raw_len as usize)
                    .map_err(|error| Error::CorruptChunk {
                        path: path.to_owned(),
                        chunk: chunk_index,
                        reason: format!("Zstd decode failed: {error}"),
                    })?,
            };

        if decoded.len() != chunk.raw_len as usize {
            return Err(Error::CorruptChunk {
                path: path.to_owned(),
                chunk: chunk_index,
                reason: format!(
                    "length mismatch: expected {}, got {}",
                    chunk.raw_len,
                    decoded.len()
                ),
            });
        }
        if xxh3_64(&decoded) != chunk.checksum {
            return Err(Error::CorruptChunk {
                path: path.to_owned(),
                chunk: chunk_index,
                reason: "checksum mismatch".into(),
            });
        }
        Ok(decoded)
    }

    fn extract_file(&self, entry: &Entry, destination: &Path) -> Result<()> {
        let target = destination.join(path_from_archive(&entry.path));
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = File::create(&target)?;
        for (chunk_index, chunk) in entry.chunks.iter().enumerate() {
            output.write_all(&self.read_chunk(&entry.path, chunk_index, chunk)?)?;
        }
        output.flush()?;
        restore_metadata(&target, entry)?;
        Ok(())
    }

    #[cfg(unix)]
    fn extract_symlink(&self, entry: &Entry, destination: &Path) -> Result<()> {
        use std::os::unix::fs::symlink;

        let target = destination.join(path_from_archive(&entry.path));
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let link_target = entry
            .symlink_target
            .as_deref()
            .ok_or_else(|| Error::InvalidFormat(format!("symlink {} has no target", entry.path)))?;
        symlink(link_target, target)?;
        Ok(())
    }

    #[cfg(not(unix))]
    fn extract_symlink(&self, entry: &Entry, _destination: &Path) -> Result<()> {
        Err(Error::InvalidFormat(format!(
            "symlink extraction is not supported on this platform: {}",
            entry.path
        )))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifySummary {
    pub entries: usize,
    pub files: usize,
    pub chunks: u64,
    pub bytes: u64,
}

fn validate_header(header: Header, file_len: u64) -> Result<()> {
    if header.version != FORMAT_VERSION {
        return Err(Error::UnsupportedVersion(header.version));
    }
    if header.flags != 0 {
        return Err(Error::InvalidFormat(format!(
            "unsupported header flags: {}",
            header.flags
        )));
    }
    if !(MIN_CHUNK_SIZE..=MAX_CHUNK_SIZE).contains(&header.chunk_size) {
        return Err(Error::InvalidFormat("invalid chunk size".into()));
    }
    if header.index_raw_len == 0
        || header.index_raw_len > MAX_INDEX_SIZE
        || header.index_stored_len == 0
        || header.index_stored_len > MAX_INDEX_SIZE
    {
        return Err(Error::InvalidFormat("invalid index size".into()));
    }
    if header.index_offset < HEADER_SIZE as u64 {
        return Err(Error::InvalidFormat("index overlaps the header".into()));
    }
    let index_end = header
        .index_offset
        .checked_add(header.index_stored_len)
        .ok_or_else(|| Error::InvalidFormat("index offset overflow".into()))?;
    if index_end != file_len {
        return Err(Error::InvalidFormat(format!(
            "archive length mismatch: index ends at {index_end}, file length is {file_len}"
        )));
    }
    Ok(())
}

fn validate_index(
    index: &ArchiveIndex,
    header: Header,
) -> Result<(HashMap<String, usize>, HashMap<String, Vec<usize>>)> {
    if index.version != header.version || index.chunk_size != header.chunk_size {
        return Err(Error::InvalidFormat(
            "header and index metadata disagree".into(),
        ));
    }

    let mut lookup = HashMap::with_capacity(index.entries.len());
    let mut children: HashMap<String, Vec<usize>> = HashMap::new();
    let mut next_payload_offset = HEADER_SIZE as u64;
    let mut previous_path: Option<&str> = None;

    for (entry_index, entry) in index.entries.iter().enumerate() {
        validate_archive_path(&entry.path)?;
        if let Some(previous) = previous_path {
            if previous >= entry.path.as_str() {
                return Err(Error::InvalidFormat(
                    "index entries must be strictly sorted by path".into(),
                ));
            }
        }
        previous_path = Some(&entry.path);

        if lookup.insert(entry.path.clone(), entry_index).is_some() {
            return Err(Error::InvalidFormat(format!(
                "duplicate path in index: {}",
                entry.path
            )));
        }
        children
            .entry(parent_path(&entry.path).to_owned())
            .or_default()
            .push(entry_index);

        match entry.kind {
            EntryKind::File => validate_file_entry(entry, header, &mut next_payload_offset)?,
            EntryKind::Directory => {
                if entry.size != 0 || !entry.chunks.is_empty() || entry.symlink_target.is_some() {
                    return Err(Error::InvalidFormat(format!(
                        "directory has file payload metadata: {}",
                        entry.path
                    )));
                }
            }
            EntryKind::Symlink => {
                if entry.size != 0 || !entry.chunks.is_empty() || entry.symlink_target.is_none() {
                    return Err(Error::InvalidFormat(format!(
                        "invalid symlink metadata: {}",
                        entry.path
                    )));
                }
            }
        }
    }

    for entry in &index.entries {
        let parent = parent_path(&entry.path);
        if parent.is_empty() {
            continue;
        }
        let parent_index = lookup.get(parent).copied().ok_or_else(|| {
            Error::InvalidFormat(format!(
                "missing parent directory {parent} for {}",
                entry.path
            ))
        })?;
        if index.entries[parent_index].kind != EntryKind::Directory {
            return Err(Error::InvalidFormat(format!(
                "parent is not a directory: {parent}"
            )));
        }
    }

    if next_payload_offset != header.index_offset {
        return Err(Error::InvalidFormat(format!(
            "payload ends at {next_payload_offset}, index starts at {}",
            header.index_offset
        )));
    }

    Ok((lookup, children))
}

fn validate_file_entry(entry: &Entry, header: Header, next_payload_offset: &mut u64) -> Result<()> {
    if entry.symlink_target.is_some() {
        return Err(Error::InvalidFormat(format!(
            "regular file has a symlink target: {}",
            entry.path
        )));
    }
    if entry.size == 0 {
        if !entry.chunks.is_empty() {
            return Err(Error::InvalidFormat(format!(
                "empty file has chunks: {}",
                entry.path
            )));
        }
        return Ok(());
    }
    if entry.chunks.is_empty() {
        return Err(Error::InvalidFormat(format!(
            "non-empty file has no chunks: {}",
            entry.path
        )));
    }

    let expected_chunks = entry.size.div_ceil(header.chunk_size as u64);
    if entry.chunks.len() as u64 != expected_chunks {
        return Err(Error::InvalidFormat(format!(
            "wrong number of chunks for {}",
            entry.path
        )));
    }

    let mut raw_total = 0_u64;
    for (index, chunk) in entry.chunks.iter().enumerate() {
        if chunk.raw_len == 0 || chunk.raw_len > header.chunk_size || chunk.stored_len == 0 {
            return Err(Error::InvalidFormat(format!(
                "invalid chunk lengths for {} chunk {index}",
                entry.path
            )));
        }
        if index + 1 != entry.chunks.len() && chunk.raw_len != header.chunk_size {
            return Err(Error::InvalidFormat(format!(
                "short non-final chunk for {} chunk {index}",
                entry.path
            )));
        }
        if chunk.codec == ChunkCodec::Stored && chunk.stored_len != chunk.raw_len {
            return Err(Error::InvalidFormat(format!(
                "stored chunk length mismatch for {} chunk {index}",
                entry.path
            )));
        }
        if chunk.offset != *next_payload_offset {
            return Err(Error::InvalidFormat(format!(
                "non-contiguous or overlapping payload for {} chunk {index}: expected offset {}, got {}",
                entry.path, *next_payload_offset, chunk.offset
            )));
        }
        let end = chunk
            .offset
            .checked_add(chunk.stored_len as u64)
            .ok_or_else(|| Error::InvalidFormat("chunk offset overflow".into()))?;
        if end > header.index_offset {
            return Err(Error::InvalidFormat(format!(
                "chunk overlaps index for {} chunk {index}",
                entry.path
            )));
        }
        *next_payload_offset = end;
        raw_total += chunk.raw_len as u64;
    }
    if raw_total != entry.size {
        return Err(Error::InvalidFormat(format!(
            "file size mismatch for {}: index says {}, chunks contain {raw_total}",
            entry.path, entry.size
        )));
    }
    Ok(())
}

fn path_from_archive(path: &str) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.split('/') {
        result.push(component);
    }
    result
}

fn restore_metadata(path: &Path, entry: &Entry) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(entry.unix_mode))?;
    }

    if let Some(modified) = entry.modified {
        let time = FileTime::from_unix_time(modified.seconds, modified.nanos);
        set_file_mtime(path, time)?;
    }
    Ok(())
}
