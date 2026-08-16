# ZSTD-Finder archive format v1 (`.zstf`)

The v1 format is intentionally small, read-only, and random-access friendly.
It is **not** `tar.zst`: files are chunked independently so reading one file or
one byte range never requires decoding earlier files.

## Layout

```text
0                    64
+--------------------+
| fixed header       |
+--------------------+
| payload chunk 0    |  stored or one independent Zstd frame
+--------------------+
| payload chunk 1    |
+--------------------+
| ...                |
+--------------------+  <- index_offset
| Zstd(JSON index)   |
+--------------------+  <- EOF
```

All integers in the fixed header are little-endian.

| Offset | Size | Field |
|---:|---:|---|
| 0 | 8 | magic `ZSTDFND1` |
| 8 | 2 | format version (`1`) |
| 10 | 2 | flags (`0` in v1) |
| 12 | 4 | logical chunk size |
| 16 | 8 | compressed index offset |
| 24 | 8 | compressed index length |
| 32 | 8 | uncompressed index length |
| 40 | 8 | XXH3-64 of uncompressed index |
| 48 | 16 | reserved, zero in v1 |

## Payload chunks

Regular files are split into fixed-size logical chunks (1 MiB by default).
Each chunk is compressed as an independent Zstd frame. If the compressed frame
would not save at least 8 bytes, the raw chunk is stored instead. This matters
for JPEG, HEIC, video, ZIP, and other already-compressed content.

The index records, for every chunk:

- archive offset;
- stored length;
- logical length;
- codec (`zstd` or `stored`);
- XXH3-64 checksum of the logical bytes.

This makes `read_at(path, offset, length)` proportional to the touched chunks,
not to the total archive size.

## Index

The index is UTF-8 JSON compressed with one Zstd frame. It contains sorted
entries for regular files, directories, and symbolic links. Paths are relative,
slash-separated, and may not contain empty, `.` or `..` components.

V1 preserves Unix mode and modification time for regular extraction. Extended
attributes, Finder tags, ACLs, hard links, sparse-file maps, and resource forks
are deliberately outside the v1 format and can be added versionably later.

## Corruption and hostile input

Readers must validate header bounds before allocation, cap index sizes, validate
all paths, reject payload ranges that overlap the header/index, and verify chunk
checksums after decoding. The reference implementation does all of these before
exposing an entry.
