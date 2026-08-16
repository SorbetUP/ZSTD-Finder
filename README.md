# ZSTD-Finder

ZSTD-Finder is a macOS-oriented, read-only archive format and engine for large
folders that must stay browsable without expanding the whole archive first.

The important distinction is that `.zstf` is **not** `tar.zst`. Every regular
file is split into independently addressable chunks, so callers can read one
photo or a byte range of one large video without decoding the files that came
before it.

## V1 status

The `feat/read-only-v1` branch contains the first read-only storage engine:

- independent Zstd chunks, 1 MiB by default;
- automatic raw storage for incompressible chunks;
- indexed lookup by path;
- random `read_at(path, offset, length)` access;
- XXH3 checksums per chunk and on the index;
- files, directories, empty files, and symbolic links;
- safe extraction and archive verification;
- CLI for pack/list/verify/read/cat/extract;
- CI on Linux and macOS in debug and release modes;
- clippy, rustfmt, and rustdoc quality gates.

The archive engine is intentionally separated from Finder mounting. The native
macOS frontend can call the same range-read API from an FSKit read-only volume,
without changing the archive format.

## CLI

```bash
cargo run --release -- pack ~/Pictures Pictures.zstf
cargo run --release -- list Pictures.zstf
cargo run --release -- verify Pictures.zstf
cargo run --release -- read Pictures.zstf 2026/photo.raw 1048576 65536 > range.bin
cargo run --release -- extract Pictures.zstf ./restored
```

Use `--chunk-size` to change random-access granularity and `--level` to change
the Zstd compression level.

## Design

See [`docs/FORMAT.md`](docs/FORMAT.md) for the v1 on-disk format and validation
rules.

## Finder integration

The target native integration is an FSKit unary file-system extension on modern
macOS. It will expose a `.zstf` archive as a read-only mounted volume in Finder
and translate file reads directly to the core `read_at` operation. The storage
engine and its corruption/seek tests land first so the filesystem layer stays a
thin adapter rather than owning archive logic.
