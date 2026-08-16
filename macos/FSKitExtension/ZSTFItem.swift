import FSKit

final class ZSTFItem: FSItem, @unchecked Sendable {
    let archivePath: String
    let identifier: FSItem.Identifier
    let itemType: FSItem.ItemType
    let size: UInt64
    let unixMode: UInt32
    let symlinkTarget: String?

    init(
        archivePath: String,
        identifier: FSItem.Identifier,
        itemType: FSItem.ItemType,
        size: UInt64,
        unixMode: UInt32,
        symlinkTarget: String? = nil
    ) {
        self.archivePath = archivePath
        self.identifier = identifier
        self.itemType = itemType
        self.size = size
        self.unixMode = unixMode
        self.symlinkTarget = symlinkTarget
        super.init()
    }

    var name: String {
        archivePath.split(separator: "/", omittingEmptySubsequences: true).last.map(String.init) ?? "/"
    }

    var parentPath: String {
        guard let separator = archivePath.lastIndex(of: "/") else { return "" }
        return String(archivePath[..<separator])
    }
}
