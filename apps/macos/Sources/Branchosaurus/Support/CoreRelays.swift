// SPDX-License-Identifier: GPL-3.0-or-later

import AppKit
import BranchosaurusCore

/// Forwards history progress from the core's worker thread.
final class ProgressRelay: HistoryObserver {
    private let handler: @Sendable (HistoryProgress) -> Void

    init(_ handler: @escaping @Sendable (HistoryProgress) -> Void) {
        self.handler = handler
    }

    func onProgress(progress: HistoryProgress) {
        handler(progress)
    }
}

/// Forwards repository changes from the core's watcher thread.
final class WatchRelay: WatchObserver {
    private let handler: @Sendable (WatchEvent) -> Void

    init(_ handler: @escaping @Sendable (WatchEvent) -> Void) {
        self.handler = handler
    }

    func onChange(event: WatchEvent) {
        handler(event)
    }
}

/// Forwards fetch/pull/push progress, and asks the person when git needs credentials.
final class RemoteRelay: RemoteObserver {
    private let handler: @Sendable (RemoteProgress) -> Void

    init(_ handler: @escaping @Sendable (RemoteProgress) -> Void) {
        self.handler = handler
    }

    func onProgress(progress: RemoteProgress) {
        handler(progress)
    }

    func askCredential(prompt: String, secret: Bool) -> String? {
        CredentialPrompt.ask(prompt, secret: secret)
    }
}

/// Asks for a username, password, passphrase or confirmation that git or ssh needs. Credential
/// helpers (the keychain) are consulted first, so this only appears when nothing is stored.
enum CredentialPrompt {
    /// Called on a core thread, which waits until the person answers.
    static func ask(_ prompt: String, secret: Bool) -> String? {
        if Thread.isMainThread {
            return MainActor.assumeIsolated { show(prompt, secret: secret) }
        }
        return DispatchQueue.main.sync {
            MainActor.assumeIsolated { show(prompt, secret: secret) }
        }
    }

    @MainActor private static func show(_ prompt: String, secret: Bool) -> String? {
        let alert = NSAlert()
        alert.messageText = secret ? "Password Required" : "Git Is Asking"
        alert.informativeText = prompt.trimmingCharacters(in: .whitespaces)
        let field = secret ? NSSecureTextField() : NSTextField()
        field.frame = NSRect(x: 0, y: 0, width: 300, height: 24)
        field.placeholderString = secret ? "Password" : nil
        alert.accessoryView = field
        alert.addButton(withTitle: "OK")
        alert.addButton(withTitle: "Cancel")
        alert.window.initialFirstResponder = field
        NSApp.activate()
        return alert.runModal() == .alertFirstButtonReturn ? field.stringValue : nil
    }
}
