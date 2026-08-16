# macOS / Finder integration

## V1 contract

The native frontend is intentionally read-only. Finder operations map to the
archive core as follows:

- lookup -> indexed path lookup;
- directory enumeration -> in-memory child index;
- attributes -> indexed metadata;
- symlink read -> indexed link target;
- file read -> `read_at(path, offset, length)` through the C ABI;
- create/write/remove/rename/link/set-attributes -> `EROFS`.

No operation extracts the complete archive as part of a normal file read.

## Native boundary

The Rust crate builds both an `rlib` and a `staticlib`. `include/zstd_finder.h`
exposes a small panic-safe C ABI. `macos/Shared/ZSTFArchiveBridge.swift` owns the
opaque archive handle and exposes indexed entries and range reads to Swift.

The filesystem source lives in `macos/FSKitExtension/` and uses a path-backed
FSKit resource to turn one `.zstf` file into one read-only volume.

## Platform split

The archive engine is tested on macOS 15 and Linux. The path-backed FSKit
adapter is compiled against macOS 26 because the SDK used for direct URL/path
resources is available there. This separation keeps the portable archive
format usable independently of the newest Finder integration API.

## Extension metadata

`Info.plist` declares `FSShortName = zstf`. `ZSTDFinderFS.entitlements` declares
the FSKit filesystem-module entitlement. CI lints both files and compiles the
Swift bridge and filesystem adapter with warnings treated as errors.

## What CI proves

CI automatically proves that:

1. archives round-trip and verify;
2. reads that cross chunk boundaries return the exact requested bytes;
3. corrupting an earlier chunk does not prevent a later independent chunk from
   being read;
4. incompressible data falls back to raw storage instead of growing;
5. truncated archives and damaged chunks are rejected;
6. the CLI performs pack/list/verify/range-read/extract end to end;
7. the C ABI opens, indexes, reads, and reports errors without unwinding;
8. the C header parses on macOS;
9. the Swift bridge and read-only FSKit adapter type-check against the macOS 26
   SDK with warnings denied;
10. the extension metadata and entitlement files are syntactically valid and
    contain the expected FSKit keys.

## What CI cannot prove yet

A GitHub-hosted runner cannot replace a real signed user installation test.
The remaining packaging milestone is a host macOS app containing the FSKit app
extension, with developer signing/activation and a real mount/unmount smoke test
on a Mac. Until that exists, the repository should not claim that a distributed
`.app` is ready for users even though the filesystem source itself compiles.
