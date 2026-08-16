import Foundation
import FSKit

final class ZSTFFileSystem: FSUnaryFileSystem, FSUnaryFileSystemOperations, @unchecked Sendable {
    private var resource: FSPathURLResource?

    func probeResource(
        resource: FSResource,
        replyHandler reply: @escaping @Sendable (FSProbeResult?, Error?) -> Void
    ) {
        guard let pathResource = resource as? FSPathURLResource,
              pathResource.url.pathExtension.lowercased() == "zstf" else {
            return reply(.notRecognized, nil)
        }

        let accessStarted = pathResource.url.startAccessingSecurityScopedResource()
        defer {
            if accessStarted {
                pathResource.url.stopAccessingSecurityScopedResource()
            }
        }

        do {
            _ = try ZSTFArchive(url: pathResource.url)
            let name = pathResource.url.deletingPathExtension().lastPathComponent
            reply(.usable(name: name, containerID: FSContainerIdentifier()), nil)
        } catch {
            reply(.notRecognized, nil)
        }
    }

    func loadResource(
        resource: FSResource,
        options: FSTaskOptions,
        replyHandler reply: @escaping @Sendable (FSVolume?, Error?) -> Void
    ) {
        guard let pathResource = resource as? FSPathURLResource else {
            return reply(nil, POSIXError(.EINVAL))
        }
        guard pathResource.url.startAccessingSecurityScopedResource() else {
            return reply(nil, POSIXError(.EACCES))
        }
        for option in options.taskOptions where option.contains("-f") {
            pathResource.url.stopAccessingSecurityScopedResource()
            return reply(nil, POSIXError(.ENOTSUP))
        }

        do {
            let volume = try ZSTFVolume(archiveURL: pathResource.url)
            self.resource = pathResource
            containerStatus = .ready
            reply(volume, nil)
        } catch {
            pathResource.url.stopAccessingSecurityScopedResource()
            reply(nil, error)
        }
    }

    func unloadResource(
        resource: FSResource,
        options: FSTaskOptions,
        replyHandler reply: @escaping @Sendable (Error?) -> Void
    ) {
        if let current = self.resource {
            current.url.stopAccessingSecurityScopedResource()
            self.resource = nil
        }
        reply(nil)
    }

    func didFinishLoading() {}
}
