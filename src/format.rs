use serde::{Deserialize, Serialize};

pub const MAGIC: [u8; 8] = *b"ZSTDFND1";
pub const FORMAT_VERSION: u16 = 1;
pub const HEADER_SIZE: usize = 64;
pub const MIN_CHUNK_SIZE: u32 = 64 * 1024;
pub const MAX_CHUNK_SIZE: u32 = 64 * 1024 * 1024;
pub const DEFAULT_CHUNK_SIZE: u32 = 1024 * 1024;
pub const MAX_INDEX_SIZE: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub version: u16,
    pub flags: u16,
    pub chunk_size: u32,
    pub index_offset: u64,
    pub index_stored_len: u64,
    pub index_raw_len: u64,
    pub index_checksum: u64,
}

impl Header {
    pub fn encode(self) -> [u8; HEADER_SIZE] {
        let mut out = [0_u8; HEADER_SIZE];
        out[0..8].copy_from_slice(&MAGIC);
        out[8..10].copy_from_slice(&self.version.to_le_bytes());
        out[10..12].copy_from_slice(&self.flags.to_le_bytes());
        out[12..16].copy_from_slice(&self.chunk_size.to_le_bytes());
        out[16..24].copy_from_slice(&self.index_offset.to_le_bytes());
        out[24..32].copy_from_slice(&self.index_stored_len.to_le_bytes());
        out[32..40].copy_from_slice(&self.index_raw_len.to_le_bytes());
        out[40..48].copy_from_slice(&self.index_checksum.to_le_bytes());
        out
    }

    pub fn decode(bytes: &[u8; HEADER_SIZE]) -> Result<Self, &'static str> {
        if bytes[0..8] != MAGIC {
            return Err("invalid ZSTD-Finder magic");
        }

        Ok(Self {
            version: u16::from_le_bytes(bytes[8..10].try_into().expect("fixed slice")),
            flags: u16::from_le_bytes(bytes[10..12].try_into().expect("fixed slice")),
            chunk_size: u32::from_le_bytes(bytes[12..16].try_into().expect("fixed slice")),
            index_offset: u64::from_le_bytes(bytes[16..24].try_into().expect("fixed slice")),
            index_stored_len: u64::from_le_bytes(bytes[24..32].try_into().expect("fixed slice")),
            index_raw_len: u64::from_le_bytes(bytes[32..40].try_into().expect("fixed slice")),
            index_checksum: u64::from_le_bytes(bytes[40..48].try_into().expect("fixed slice")),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArchiveIndex {
    pub version: u16,
    pub chunk_size: u32,
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    pub kind: EntryKind,
    pub size: u64,
    pub unix_mode: u32,
    pub modified: Option<UnixTime>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chunks: Vec<ChunkRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symlink_target: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    File,
    Directory,
    Symlink,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnixTime {
    pub seconds: i64,
    pub nanos: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChunkRef {
    pub offset: u64,
    pub stored_len: u32,
    pub raw_len: u32,
    pub codec: ChunkCodec,
    pub checksum: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChunkCodec {
    Stored,
    Zstd,
}
