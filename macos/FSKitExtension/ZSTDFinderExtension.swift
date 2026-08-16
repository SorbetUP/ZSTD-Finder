import FSKit

@main
struct ZSTDFinderExtension: UnaryFileSystemExtension {
    let fileSystem = ZSTFFileSystem()
}
