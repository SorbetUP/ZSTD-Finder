# ZSTD-Finder

ZSTD-Finder is a macOS-oriented, read-only archive format and filesystem adapter
for large folders that must stay browsable without expanding the whole archive.

A `.zstf` archive is deliberately **not** `tar.zst`. Every regular file is split
into independently addressable chunks, so Finder can request one photo or a byte
range of one large video without decoding the files or chunks that came before
it.

## V1 status

The `feat/read-only-v1` branch contains the first read-only implementation:

- independent Zstd chunks, 1 MiB by default;
- automatic raw storage for incompressible chunks;
- indexed path lookup and random `read_at(path, offset, length)` access;
- XXH3 checksums per chunk and on the index;
- files, directories, empty files, and symbolic links;
- bounds/path validation, safe extraction, and full archive verification;
- CLI for `pack`, `list`, `verify`, `read`, `cat`, and `extract`;
- panic-safe C ABI and Swift bridge for native macOS code;
- read-only FSKit volume adapter: lookup, directory enumeration, attributes,
  symlinks, and random file reads; every mutation returns `EROFS`;
- CI tests in debug and release on Linux and macOS 15;
- native C/Swift/FSKit compile gate on macOS 26;
- `rustfmt`, Clippy with warnings denied, Rustdoc, and extension metadata gates.

The path-backed FSKit frontend currently targets **macOS 26+**. The storage
engine itself is independent of FSKit and remains tested on macOS 15 as well.

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

## Architecture

```text
.zstf archive
    |
    +-- Rust archive engine: index + independently compressed chunks
    |       |
    |       +-- read_at(path, offset, length)
    |
    +-- stable C ABI
            |
            +-- Swift bridge
                    |
                    +-- read-only FSKit volume
                            |
                            +-- Finder / Quick Look / applications
```

See [`docs/FORMAT.md`](docs/FORMAT.md) for the v1 on-disk format and
[`docs/MACOS.md`](docs/MACOS.md) for the native integration boundary.

## CI policy

Every feature-branch push is gated by the portable archive tests, debug and
release test suites, formatting, Clippy with warnings denied, Rustdoc, and the
macOS 26 native FSKit/C/Swift compatibility checks. The random-access tests
explicitly verify that damage to an earlier chunk does not force a later chunk
to be decoded or read.

## Installation status

The V1 filesystem implementation and its extension metadata are present and are
compiled in CI. The repository does **not yet ship a signed installable host
`.app`/`.appex` bundle**. Installing an FSKit module requires Apple extension
packaging, entitlements, code signing, and activation on the target Mac. Keeping
that packaging step separate prevents CI from claiming that Finder mounting was
runtime-tested when the runner cannot perform a real user installation.
