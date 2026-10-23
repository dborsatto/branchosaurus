// SPDX-License-Identifier: GPL-3.0-or-later

import AppKit
import BranchosaurusCore

/// Authors' pictures for the commit graph. The core says where a picture may be (Gravatar, or
/// GitHub for its no-reply addresses); this downloads them, keeps them on disk, and falls back to
/// initials. Downloading can be turned off in Settings.
@MainActor
final class AvatarStore {
    static let shared = AvatarStore()
    /// Posted when pictures arrive, or when downloading is switched on or off.
    static let didChange = Notification.Name("AvatarStoreDidChange")
    static let loadImagesKey = "loadAvatars"
    /// Pictures are drawn at most this big, in points.
    static let pointSize: CGFloat = 24
    private static let maxDownloads = 6

    struct Author {
        let initials: String
        let color: UInt16
        let image: NSImage?
    }

    private enum Picture {
        case pending(URL)
        case loading
        case loaded(NSImage)
        case missing
    }

    private var avatars: [String: Avatar] = [:]
    private var pictures: [String: Picture] = [:]
    /// Waiting downloads, newest last: what was scrolled to most recently loads first.
    private var queue: [String] = []
    private var downloads = 0
    private var loadsImages: Bool

    private let session: URLSession = {
        let configuration = URLSessionConfiguration.default
        let caches = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
            .appendingPathComponent(Bundle.main.bundleIdentifier ?? "Branchosaurus")
            .appendingPathComponent("Avatars")
        configuration.urlCache = URLCache(memoryCapacity: 4 << 20, diskCapacity: 64 << 20, directory: caches)
        // Pictures rarely change; a stale one is fine.
        configuration.requestCachePolicy = .returnCacheDataElseLoad
        configuration.httpMaximumConnectionsPerHost = maxDownloads
        configuration.timeoutIntervalForRequest = 15
        return URLSession(configuration: configuration)
    }()

    private init() {
        UserDefaults.standard.register(defaults: [Self.loadImagesKey: true])
        loadsImages = UserDefaults.standard.bool(forKey: Self.loadImagesKey)
        _ = NotificationCenter.default.addObserver(
            forName: UserDefaults.didChangeNotification, object: nil, queue: .main
        ) { _ in
            MainActor.assumeIsolated { AvatarStore.shared.settingsChanged() }
        }
    }

    /// The author's picture if it has arrived, and their initials and color either way. Asks for
    /// the picture the first time an email is seen.
    func author(name: String, email: String) -> Author {
        let key = email.lowercased()
        let avatar =
            avatars[key]
            ?? {
                let made = BranchosaurusCore.avatar(
                    name: name, email: email, pixels: UInt32(Self.pointSize * 3))
                avatars[key] = made
                return made
            }()
        var image: NSImage?
        switch pictures[key] {
        case let .loaded(loaded): image = loaded
        case .pending, .loading, .missing: break
        case nil:
            if let url = avatar.imageUrl.flatMap(URL.init(string:)) {
                pictures[key] = .pending(url)
                queue.append(key)
                startDownloads()
            } else {
                pictures[key] = .missing
            }
        }
        return Author(initials: avatar.initials, color: avatar.color, image: loadsImages ? image : nil)
    }

    private func startDownloads() {
        while loadsImages, downloads < Self.maxDownloads, let key = queue.popLast() {
            guard case let .pending(url) = pictures[key] else { continue }
            pictures[key] = .loading
            downloads += 1
            Task {
                let image = await download(url)
                downloads -= 1
                pictures[key] = image.map(Picture.loaded) ?? .missing
                if image != nil {
                    NotificationCenter.default.post(name: Self.didChange, object: self)
                }
                startDownloads()
            }
        }
    }

    private func download(_ url: URL) async -> NSImage? {
        guard let (data, response) = try? await session.data(from: url),
            (response as? HTTPURLResponse)?.statusCode == 200
        else { return nil }
        return NSImage(data: data)
    }

    private func settingsChanged() {
        let enabled = UserDefaults.standard.bool(forKey: Self.loadImagesKey)
        guard enabled != loadsImages else { return }
        loadsImages = enabled
        startDownloads()
        NotificationCenter.default.post(name: Self.didChange, object: self)
    }
}
