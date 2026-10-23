// SPDX-License-Identifier: GPL-3.0-or-later

import BranchosaurusCore
import Foundation
import Testing

/// Smoke tests of the generated bindings: the Rust core is covered by its own test suite, these
/// check that data crosses the FFI boundary intact.
@Suite struct BindingsTests {
    @Test func historyStatusAndDiffsCrossTheBoundary() throws {
        let fixture = try Fixture()
        try fixture.write("README.md", "hello\n")
        try fixture.git("add", "README.md")
        try fixture.git("commit", "-m", "first")
        try fixture.write("README.md", "hello\nworld\n")
        try fixture.git("commit", "-am", "second")
        try fixture.write("README.md", "hello\nthere\n")
        try fixture.write("new file.txt", "new\n")

        let repository = try Repository.discover(path: fixture.url.path)
        let info = try repository.info()
        #expect(info.head == .branch(name: "main", target: try fixture.git("rev-parse", "HEAD")))

        let history = try repository.loadHistory(observer: IgnoreProgress(), cancel: CancelToken())
        #expect(history.rowCount() == 2)
        let rows = try history.rows(start: 0, count: 10)
        #expect(rows.map(\.summary) == ["second", "first"])
        #expect(rows[0].refs.first?.name == "main")
        #expect(rows[0].refs.first?.isHead == true)
        #expect(rows[0].refs.first?.local == true)
        #expect(rows[0].commitTime >= rows[1].commitTime && rows[1].commitTime > 0)
        #expect(history.rowOf(id: rows[1].id) == 1)

        let details = try repository.commitDetails(id: rows[0].id)
        #expect(details.changes.map(\.path.display) == ["README.md"])
        #expect(details.changes.first?.stats == LineStats(additions: 1, deletions: 0))
        let commitDiff = try repository.commitFileDiff(change: details.changes[0])
        guard case let .text(hunks, additions, deletions) = commitDiff.content else {
            Issue.record("expected a text diff")
            return
        }
        #expect((additions, deletions) == (1, 0))
        #expect(hunks.first?.lines.last?.text == "world")

        let status = try repository.status()
        #expect(status.unstaged.map(\.path.display) == ["README.md", "new file.txt"])
        #expect(status.unstaged.map(\.kind) == [.modified, .untracked])
        let unstaged = try repository.unstagedDiff(entry: status.unstaged[0])
        guard case let .text(_, added, removed) = unstaged.content else {
            Issue.record("expected a text diff")
            return
        }
        #expect((added, removed) == (1, 1))
    }

    @Test func highlightsAreUTF16Ranges() throws {
        let fixture = try Fixture()
        try fixture.write("main.rs", "let s = \"☕𝄞\"; // ünï\n")
        let repository = try Repository.discover(path: fixture.url.path)
        let entry = try #require(try repository.status().unstaged.first)
        let diff = try repository.unstagedDiff(entry: entry)
        #expect(diff.language == "Rust")
        guard case let .text(hunks, _, _) = diff.content, let line = hunks.first?.lines.first else {
            Issue.record("expected a text diff")
            return
        }
        let text = line.text as NSString
        let piece = { (kind: HighlightKind) in
            line.highlights.first { $0.kind == kind }.map {
                text.substring(with: NSRange(location: Int($0.start), length: Int($0.end - $0.start)))
            }
        }
        #expect(piece(.string) == "\"☕𝄞\"")
        #expect(piece(.comment) == "// ünï")
    }

    @Test func avatarsHaveInitialsAndPictureAddresses() {
        let github = avatar(name: "The Octocat", email: "583231+octocat@users.noreply.github.com", pixels: 48)
        #expect(github.initials == "TO")
        #expect(github.imageUrl == "https://avatars.githubusercontent.com/u/583231?s=48&v=4")
        let gravatar = avatar(name: "Ada Lovelace", email: "ada@example.com", pixels: 48)
        #expect(gravatar.imageUrl?.hasPrefix("https://gravatar.com/avatar/") == true)
        #expect(avatar(name: "", email: "", pixels: 48).imageUrl == nil)
    }

    @Test func errorsCarryTheCoreMessage() {
        let missing = FileManager.default.temporaryDirectory
            .appendingPathComponent("branchosaurus-missing-\(UUID().uuidString)")
        #expect {
            try Repository.discover(path: missing.path)
        } throws: { error in
            guard let error = error as? BranchosaurusError, case .NotARepository = error else { return false }
            return error.message.contains("no git repository")
        }
    }

    @Test func cancellingStopsHistoryLoading() throws {
        let fixture = try Fixture()
        try fixture.git("commit", "--allow-empty", "-m", "only")
        let repository = try Repository.discover(path: fixture.url.path)
        let cancel = CancelToken()
        cancel.cancel()
        #expect(throws: BranchosaurusError.self) {
            try repository.loadHistory(observer: IgnoreProgress(), cancel: cancel)
        }
    }
}

@Suite struct WriteBindingsTests {
    @Test func linesCommitsAndBranches() throws {
        let fixture = try Fixture()
        try fixture.write("a.txt", "one\ntwo\n")
        try fixture.git("add", "a.txt")
        try fixture.git("commit", "-m", "first")
        try fixture.write("a.txt", "one\nTWO\ntwo\nthree\n")

        let repository = try Repository.discover(path: fixture.url.path)
        let entry = try #require(try repository.status().unstaged.first)
        try repository.stageLines(entry: entry, selection: LineSelection(removed: [], added: [4]))
        #expect(try fixture.git("show", ":a.txt") == "one\ntwo\nthree")

        let commit = try repository.commit(message: "Add three", amend: false)
        #expect(commit == (try fixture.git("rev-parse", "HEAD")))
        #expect(try repository.status().unstaged.map(\.path.display) == ["a.txt"])

        try repository.createBranch(name: "feature", start: nil, checkout: true)
        #expect(try repository.info().head == .branch(name: "feature", target: commit))
        try repository.checkout(target: .localBranch(name: "main"))
        #expect(try repository.info().head == .branch(name: "main", target: commit))

        #expect(isValidBranchName(name: "feature/x"))
        #expect(!isValidBranchName(name: "bad name"))
        #expect {
            try repository.createBranch(name: "bad name", start: nil, checkout: false)
        } throws: { error in
            guard case .Unsupported = error as? BranchosaurusError else { return false }
            return true
        }
    }

    @Test func pushToANewUpstream() throws {
        let remote = try Fixture(bare: true)
        let fixture = try Fixture()
        try fixture.git("commit", "--allow-empty", "-m", "first")
        try fixture.git("remote", "add", "origin", remote.url.path)

        let repository = try Repository.discover(path: fixture.url.path)
        let observer = RecordingObserver()
        let target = try repository.push(force: false, observer: observer, cancel: CancelToken())
        #expect(target.remote == "origin")
        #expect(target.remoteRef == "refs/heads/main")
        #expect(target.setUpstream)
        #expect(try remote.git("rev-parse", "main") == (try fixture.git("rev-parse", "HEAD")))
        #expect(try !repository.pushTarget().setUpstream)
        #expect(observer.credentialPrompts.isEmpty)
    }
}

/// Records progress; refuses credential prompts.
final class RecordingObserver: RemoteObserver, @unchecked Sendable {
    private let lock = NSLock()
    private var prompts: [String] = []

    var credentialPrompts: [String] { lock.withLock { prompts } }

    func onProgress(progress: RemoteProgress) {}

    func askCredential(prompt: String, secret: Bool) -> String? {
        lock.withLock { prompts.append(prompt) }
        return nil
    }
}

final class IgnoreProgress: HistoryObserver {
    func onProgress(progress: HistoryProgress) {}
}

/// A throwaway repository, isolated from the user's git configuration.
struct Fixture {
    let url: URL

    /// The identity and isolation the core's own git calls see as well (it inherits this
    /// process's environment): no system or user configuration, so no signing, hooks or helpers.
    static let environment = [
        "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_CONFIG_GLOBAL": "/dev/null",
        "GIT_AUTHOR_NAME": "Ada Lovelace",
        "GIT_AUTHOR_EMAIL": "ada@example.com",
        "GIT_COMMITTER_NAME": "Ada Lovelace",
        "GIT_COMMITTER_EMAIL": "ada@example.com",
    ]

    init(bare: Bool = false) throws {
        for (key, value) in Self.environment {
            setenv(key, value, 1)
        }
        url = FileManager.default.temporaryDirectory
            .appendingPathComponent("branchosaurus-tests-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        if bare {
            try git("init", "--quiet", "--bare", "--initial-branch=main")
        } else {
            try git("init", "--quiet", "--initial-branch=main")
        }
    }

    func write(_ path: String, _ contents: String) throws {
        try contents.write(to: url.appendingPathComponent(path), atomically: true, encoding: .utf8)
    }

    @discardableResult
    func git(_ arguments: String...) throws -> String {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
        process.arguments = ["git"] + arguments
        process.currentDirectoryURL = url
        process.environment = ProcessInfo.processInfo.environment.merging(Self.environment) { $1 }
        let output = Pipe()
        process.standardOutput = output
        try process.run()
        process.waitUntilExit()
        let text = String(decoding: output.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
        guard process.terminationStatus == 0 else {
            throw FixtureError.gitFailed(arguments.joined(separator: " "))
        }
        return text.trimmingCharacters(in: .whitespacesAndNewlines)
    }
}

enum FixtureError: Error {
    case gitFailed(String)
}
