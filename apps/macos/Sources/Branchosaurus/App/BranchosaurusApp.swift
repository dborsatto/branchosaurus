// SPDX-License-Identifier: GPL-3.0-or-later

import AppKit
import SwiftUI

@main
struct BranchosaurusApp: App {
    init() {
        #if DEBUG
            DebugSnapshot.scheduleIfRequested()
        #endif
    }

    var body: some Scene {
        // One window per repository. A window without a repository shows the welcome screen.
        WindowGroup("Branchosaurus", for: URL.self) { $url in
            WindowRoot(url: $url)
        }
        // Folders opened from Finder or the Dock arrive as external events.
        .handlesExternalEvents(matching: ["*"])
        .defaultSize(width: 1200, height: 780)
        .commands {
            RepositoryCommands()
        }

        Settings {
            SettingsView()
        }
    }
}

/// Shows a repository, or the welcome screen, and handles folders opened from Finder or the Dock.
struct WindowRoot: View {
    @Binding var url: URL?
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Group {
            if let url {
                RepositoryWindow(url: url)
                    .id(url)
            } else {
                WelcomeView(url: $url)
            }
        }
        .onOpenURL { opened in
            guard opened.isFileURL, opened.hasDirectoryPath || isDirectory(opened) else { return }
            let opened = opened.standardizedFileURL
            RecentRepositories.add(opened)
            if url == nil {
                url = opened
            } else if opened != url {
                openWindow(value: opened)
            }
        }
    }

    private func isDirectory(_ url: URL) -> Bool {
        (try? url.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) ?? false
    }
}

struct RepositoryCommands: Commands {
    @Environment(\.openWindow) private var openWindow

    var body: some Commands {
        CommandGroup(replacing: .newItem) {
            Button("Open Repository…") {
                if let url = chooseRepositoryFolder() {
                    RecentRepositories.add(url)
                    openWindow(value: url)
                }
            }
            .keyboardShortcut("o")
        }
    }
}

/// Asks for a folder. Whether it is a repository is checked when it is opened.
@MainActor
func chooseRepositoryFolder() -> URL? {
    let panel = NSOpenPanel()
    panel.canChooseDirectories = true
    panel.canChooseFiles = false
    panel.allowsMultipleSelection = false
    panel.prompt = "Open"
    panel.message = "Choose a Git repository"
    return panel.runModal() == .OK ? panel.url : nil
}

/// Repositories opened recently, most recent first.
enum RecentRepositories {
    private static let key = "recentRepositories"
    private static let limit = 10

    static func load() -> [URL] {
        let paths = UserDefaults.standard.stringArray(forKey: key) ?? []
        return paths.map { URL(fileURLWithPath: $0) }
            .filter { FileManager.default.fileExists(atPath: $0.path) }
    }

    static func add(_ url: URL) {
        var paths = UserDefaults.standard.stringArray(forKey: key) ?? []
        paths.removeAll { $0 == url.path }
        paths.insert(url.path, at: 0)
        UserDefaults.standard.set(Array(paths.prefix(limit)), forKey: key)
    }
}

/// A repository given on the command line as `-repository <path>`, e.g.
/// `open Branchosaurus.app --args -repository ~/code/repo`. (A bare path would be taken by AppKit
/// as a document to open, which this app does not handle yet.)
enum LaunchArguments {
    @MainActor static var repositoryURL: URL? = {
        guard let path = UserDefaults.standard.string(forKey: "repository") else { return nil }
        let url = URL(fileURLWithPath: (path as NSString).expandingTildeInPath).standardizedFileURL
        var isDirectory: ObjCBool = false
        guard FileManager.default.fileExists(atPath: url.path, isDirectory: &isDirectory),
            isDirectory.boolValue
        else { return nil }
        return url
    }()

    /// The launch argument is used by the first window only.
    @MainActor static func take() -> URL? {
        defer { repositoryURL = nil }
        return repositoryURL
    }
}
