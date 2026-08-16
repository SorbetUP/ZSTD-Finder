import Darwin
import Foundation
import FSKit

final class ZSTFVolume: FSVolume, FSVolume.Operations, FSVolume.ReadWriteOperations, @unchecked Sendable {
    private let archive: ZSTFArchive
    private let rootItem: ZSTFItem
    private let itemsByPath: [String: ZSTFItem]
    private let childrenByPath: [String: [ZSTFItem]]
    private let logicalBytes: UInt64

    init(archiveURL: URL) throws {
        archive = try ZSTFArchive(url: archiveURL)
        rootItem = ZSTFItem(
            archivePath: "",
            identifier: .rootDirectory,
            itemType: .directory,
            size: 0,
            unixMode: 0o40555
        )

        let entries = try archive.entries()
        var items: [String: ZSTFItem] = ["": rootItem]
        var children: [String: [ZSTFItem]] = [:]
        var totalBytes: UInt64 = 0

        for entry in entries {
            let idRaw = UInt64(entry.index) + 1_000
            guard let identifier = FSItem.Identifier(rawValue: idRaw) else {
                throw POSIXError(.EINVAL)
            }
            let itemType: FSItem.ItemType
            switch entry.kind {
            case .file:
                itemType = .file
                totalBytes = totalBytes &+ entry.size
            case .directory:
                itemType = .directory
            case .symlink:
                itemType = .symlink
            }
            let item = ZSTFItem(
                archivePath: entry.path,
                identifier: identifier,
                itemType: itemType,
                size: entry.size,
                unixMode: entry.unixMode,
                symlinkTarget: entry.symlinkTarget
            )
            items[entry.path] = item
            children[item.parentPath, default: []].append(item)
        }

        for key in children.keys {
            children[key]?.sort { $0.name < $1.name }
        }

        itemsByPath = items
        childrenByPath = children
        logicalBytes = totalBytes

        let volumeName = archiveURL.deletingPathExtension().lastPathComponent
        super.init(volumeID: FSVolume.Identifier(), volumeName: FSFileName(string: volumeName))
    }

    // MARK: - FSVolume.PathConfOperations

    var maximumLinkCount: Int { 1 }
    var maximumNameLength: Int { 255 }
    var restrictsOwnershipChanges: Bool { true }
    var truncatesLongNames: Bool { false }
    var maximumFileSize: UInt64 { UInt64.max }
    var maximumXattrSize: Int { 0 }

    // MARK: - Volume properties

    var supportedVolumeCapabilities: FSVolume.SupportedCapabilities {
        let capabilities = FSVolume.SupportedCapabilities()
        capabilities.supports64BitObjectIDs = true
        capabilities.supportsPersistentObjectIDs = false
        capabilities.supportsSymbolicLinks = true
        capabilities.supportsHardLinks = false
        capabilities.caseFormat = .sensitive
        return capabilities
    }

    var volumeStatistics: FSStatFSResult {
        let stats = FSStatFSResult(fileSystemTypeName: "zstf")
        stats.blockSize = 4_096
        stats.ioSize = 1_048_576
        stats.totalBytes = logicalBytes
        stats.usedBytes = logicalBytes
        stats.freeBytes = 0
        stats.availableBytes = 0
        stats.totalFiles = UInt64(itemsByPath.count)
        stats.freeFiles = 0
        return stats
    }

    // MARK: - Lifecycle

    func activate(
        options: FSTaskOptions,
        replyHandler reply: @escaping @Sendable (FSItem?, Error?) -> Void
    ) {
        reply(rootItem, nil)
    }

    func deactivate(
        options: FSDeactivateOptions,
        replyHandler reply: @escaping @Sendable (Error?) -> Void
    ) {
        reply(nil)
    }

    func mount(
        options: FSTaskOptions,
        replyHandler reply: @escaping @Sendable (Error?) -> Void
    ) {
        reply(nil)
    }

    func unmount(replyHandler reply: @escaping @Sendable () -> Void) {
        reply()
    }

    func synchronize(
        flags: FSSyncFlags,
        replyHandler reply: @escaping @Sendable (Error?) -> Void
    ) {
        reply(nil)
    }

    // MARK: - Lookup and enumeration

    func lookupItem(
        named name: FSFileName,
        inDirectory directory: FSItem,
        replyHandler reply: @escaping @Sendable (FSItem?, FSFileName?, Error?) -> Void
    ) {
        guard let directory = directory as? ZSTFItem, directory.itemType == .directory,
              let component = name.string else {
            return reply(nil, nil, POSIXError(.EINVAL))
        }
        let path = directory.archivePath.isEmpty ? component : "\(directory.archivePath)/\(component)"
        guard let item = itemsByPath[path] else {
            return reply(nil, nil, POSIXError(.ENOENT))
        }
        reply(item, FSFileName(string: item.name), nil)
    }

    func enumerateDirectory(
        _ directory: FSItem,
        startingAt cookie: FSDirectoryCookie,
        verifier: FSDirectoryVerifier,
        attributes attributesRequest: FSItem.GetAttributesRequest?,
        packer: FSDirectoryEntryPacker,
        replyHandler reply: @escaping @Sendable (FSDirectoryVerifier, Error?) -> Void
    ) {
        guard let directory = directory as? ZSTFItem, directory.itemType == .directory else {
            return reply(verifier, POSIXError(.ENOTDIR))
        }

        let children = childrenByPath[directory.archivePath] ?? []
        let start = Int(cookie.rawValue)
        guard start <= children.count else {
            return reply(verifier, POSIXError(.EINVAL))
        }

        for index in start..<children.count {
            let item = children[index]
            let attributes = attributesRequest == nil ? nil : attributes(for: item)
            let nextCookie = FSDirectoryCookie(rawValue: UInt64(index + 1))
            if !packer.packEntry(
                name: FSFileName(string: item.name),
                itemType: item.itemType,
                itemID: item.identifier,
                nextCookie: nextCookie,
                attributes: attributes
            ) {
                break
            }
        }

        reply(FSDirectoryVerifier(rawValue: 1), nil)
    }

    // MARK: - Attributes and links

    func getAttributes(
        _ requestedAttributes: FSItem.GetAttributesRequest,
        of item: FSItem,
        replyHandler reply: @escaping @Sendable (FSItem.Attributes?, Error?) -> Void
    ) {
        guard let item = item as? ZSTFItem else {
            return reply(nil, POSIXError(.EINVAL))
        }
        reply(attributes(for: item), nil)
    }

    func setAttributes(
        _ newAttributes: FSItem.SetAttributesRequest,
        on item: FSItem,
        replyHandler reply: @escaping @Sendable (FSItem.Attributes?, Error?) -> Void
    ) {
        reply(nil, readOnlyError())
    }

    func readSymbolicLink(
        _ item: FSItem,
        replyHandler reply: @escaping @Sendable (FSFileName?, Error?) -> Void
    ) {
        guard let item = item as? ZSTFItem,
              item.itemType == .symlink,
              let target = item.symlinkTarget else {
            return reply(nil, POSIXError(.EINVAL))
        }
        reply(FSFileName(string: target), nil)
    }

    // MARK: - Random-access file reads

    func read(
        from item: FSItem,
        at offset: off_t,
        length: Int,
        into buffer: FSMutableFileDataBuffer,
        replyHandler reply: @escaping @Sendable (Int, Error?) -> Void
    ) {
        guard let item = item as? ZSTFItem, item.itemType == .file, offset >= 0, length >= 0 else {
            return reply(0, POSIXError(.EINVAL))
        }

        do {
            let data = try archive.read(path: item.archivePath, offset: UInt64(offset), length: length)
            buffer.withUnsafeMutableBytes { destination in
                data.withUnsafeBytes { source in
                    guard let sourceBase = source.baseAddress,
                          let destinationBase = destination.baseAddress,
                          !data.isEmpty else { return }
                    destinationBase.copyMemory(from: sourceBase, byteCount: data.count)
                }
            }
            reply(data.count, nil)
        } catch {
            reply(0, error)
        }
    }

    func write(
        contents data: Data,
        to item: FSItem,
        at offset: off_t,
        replyHandler reply: @escaping @Sendable (Int, Error?) -> Void
    ) {
        reply(0, readOnlyError())
    }

    // MARK: - Mutations are intentionally rejected in V1

    func createItem(
        named name: FSFileName,
        type: FSItem.ItemType,
        inDirectory directory: FSItem,
        attributes newAttributes: FSItem.SetAttributesRequest,
        replyHandler reply: @escaping @Sendable (FSItem?, FSFileName?, Error?) -> Void
    ) {
        reply(nil, nil, readOnlyError())
    }

    func removeItem(
        _ item: FSItem,
        named name: FSFileName,
        fromDirectory directory: FSItem,
        replyHandler reply: @escaping @Sendable (Error?) -> Void
    ) {
        reply(readOnlyError())
    }

    func renameItem(
        _ item: FSItem,
        inDirectory sourceDirectory: FSItem,
        named sourceName: FSFileName,
        to destinationName: FSFileName,
        inDirectory destinationDirectory: FSItem,
        overItem: FSItem?,
        replyHandler reply: @escaping @Sendable (FSFileName?, Error?) -> Void
    ) {
        reply(nil, readOnlyError())
    }

    func createLink(
        to item: FSItem,
        named name: FSFileName,
        inDirectory directory: FSItem,
        replyHandler reply: @escaping @Sendable (FSFileName?, Error?) -> Void
    ) {
        reply(nil, readOnlyError())
    }

    func createSymbolicLink(
        named name: FSFileName,
        inDirectory directory: FSItem,
        attributes newAttributes: FSItem.SetAttributesRequest,
        linkContents: FSFileName,
        replyHandler reply: @escaping @Sendable (FSItem?, FSFileName?, Error?) -> Void
    ) {
        reply(nil, nil, readOnlyError())
    }

    func reclaimItem(
        _ item: FSItem,
        replyHandler reply: @escaping @Sendable (Error?) -> Void
    ) {
        reply(nil)
    }

    private func attributes(for item: ZSTFItem) -> FSItem.Attributes {
        let result = FSItem.Attributes()
        result.fileID = item.identifier
        result.parentID = itemsByPath[item.parentPath]?.identifier ?? .parentOfRoot
        result.type = item.itemType
        result.mode = item.unixMode
        result.linkCount = 1
        result.uid = UInt32(getuid())
        result.gid = UInt32(getgid())
        result.flags = 0
        result.size = item.itemType == .symlink ? UInt64(item.symlinkTarget?.utf8.count ?? 0) : item.size
        result.allocSize = result.size
        return result
    }

    private func readOnlyError() -> Error {
        POSIXError(.EROFS)
    }
}
