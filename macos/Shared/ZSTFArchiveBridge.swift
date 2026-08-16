import Foundation

struct ZSTFEntry: Sendable {
    enum Kind: Sendable {
        case file
        case directory
        case symlink
    }

    let index: Int
    let path: String
    let kind: Kind
    let size: UInt64
    let unixMode: UInt32
    let modifiedSeconds: Int64?
    let modifiedNanos: UInt32?
    let symlinkTarget: String?
}

final class ZSTFArchive: @unchecked Sendable {
    private var handle: OpaquePointer?

    init(url: URL) throws {
        var opened: OpaquePointer?
        let result = url.path.withCString { path in
            zstf_archive_open(path, &opened)
        }
        guard result == ZSTF_OK, let opened else {
            throw Self.nativeError(code: result)
        }
        self.handle = opened
    }

    deinit {
        if let handle {
            zstf_archive_close(handle)
        }
    }

    func entries() throws -> [ZSTFEntry] {
        let handle = try liveHandle()
        var count = 0
        let result = zstf_archive_entry_count(handle, &count)
        guard result == ZSTF_OK else {
            throw Self.nativeError(code: result)
        }
        return try (0..<count).map { index in
            let path = try copyIndexedString(function: zstf_archive_entry_path, index: index)
            var metadata = ZstfEntryMetadata()
            let metadataResult = zstf_archive_entry_metadata(handle, index, &metadata)
            guard metadataResult == ZSTF_OK else {
                throw Self.nativeError(code: metadataResult)
            }

            let kind: ZSTFEntry.Kind
            let symlinkTarget: String?
            switch metadata.kind {
            case UInt32(ZSTF_KIND_FILE):
                kind = .file
                symlinkTarget = nil
            case UInt32(ZSTF_KIND_DIRECTORY):
                kind = .directory
                symlinkTarget = nil
            case UInt32(ZSTF_KIND_SYMLINK):
                kind = .symlink
                symlinkTarget = try copyIndexedString(
                    function: zstf_archive_entry_symlink_target,
                    index: index
                )
            default:
                throw NSError(
                    domain: "ZSTDFinder.Native",
                    code: Int(ZSTF_ERR_ARCHIVE),
                    userInfo: [NSLocalizedDescriptionKey: "Unknown archive entry kind \(metadata.kind)"]
                )
            }

            return ZSTFEntry(
                index: index,
                path: path,
                kind: kind,
                size: metadata.size,
                unixMode: metadata.unix_mode,
                modifiedSeconds: metadata.has_modified == 0 ? nil : metadata.modified_seconds,
                modifiedNanos: metadata.has_modified == 0 ? nil : metadata.modified_nanos,
                symlinkTarget: symlinkTarget
            )
        }
    }

    func read(path: String, offset: UInt64, length: Int) throws -> Data {
        guard length >= 0 else {
            throw CocoaError(.fileReadInvalidFileName)
        }
        let handle = try liveHandle()
        var data = Data(count: length)
        var bytesRead = 0
        let result = path.withCString { pathPointer in
            data.withUnsafeMutableBytes { rawBuffer in
                zstf_archive_read(
                    handle,
                    pathPointer,
                    offset,
                    rawBuffer.baseAddress?.assumingMemoryBound(to: UInt8.self),
                    length,
                    &bytesRead
                )
            }
        }
        guard result == ZSTF_OK else {
            throw Self.nativeError(code: result)
        }
        if bytesRead < data.count {
            data.removeSubrange(bytesRead..<data.count)
        }
        return data
    }

    private func liveHandle() throws -> OpaquePointer {
        guard let handle else {
            throw NSError(
                domain: "ZSTDFinder.Native",
                code: Int(ZSTF_ERR_INVALID_ARGUMENT),
                userInfo: [NSLocalizedDescriptionKey: "Archive handle is closed"]
            )
        }
        return handle
    }

    private func copyIndexedString(
        function: (
            OpaquePointer?,
            Int,
            UnsafeMutablePointer<UInt8>?,
            Int,
            UnsafeMutablePointer<Int>?
        ) -> Int32,
        index: Int
    ) throws -> String {
        let handle = try liveHandle()
        var length = 0
        var result = function(handle, index, nil, 0, &length)
        guard result == ZSTF_OK else {
            throw Self.nativeError(code: result)
        }
        if length == 0 {
            return ""
        }
        var bytes = [UInt8](repeating: 0, count: length)
        result = bytes.withUnsafeMutableBufferPointer { buffer in
            function(handle, index, buffer.baseAddress, buffer.count, &length)
        }
        guard result == ZSTF_OK else {
            throw Self.nativeError(code: result)
        }
        guard let value = String(bytes: bytes.prefix(length), encoding: .utf8) else {
            throw CocoaError(.fileReadInapplicableStringEncoding)
        }
        return value
    }

    private static func nativeError(code: Int32) -> Error {
        let required = zstf_last_error(nil, 0)
        var buffer = [CChar](repeating: 0, count: required + 1)
        _ = buffer.withUnsafeMutableBufferPointer { pointer in
            zstf_last_error(pointer.baseAddress, pointer.count)
        }
        let message = buffer.withUnsafeBufferPointer { pointer -> String in
            guard let baseAddress = pointer.baseAddress else { return "ZSTD-Finder native error" }
            return String(cString: baseAddress)
        }
        return NSError(
            domain: "ZSTDFinder.Native",
            code: Int(code),
            userInfo: [NSLocalizedDescriptionKey: message.isEmpty ? "ZSTD-Finder native error" : message]
        )
    }
}
