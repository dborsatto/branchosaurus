// SPDX-License-Identifier: GPL-3.0-or-later

import BranchosaurusCore
import Foundation

/// What a hunk or line action does.
enum LineAction {
    case stage, unstage, discard
}

extension RepositoryModel {
    // MARK: Staging

    func stage(_ entries: [StatusEntry]) async {
        await perform("Couldn’t Stage") { try $0.stage(entries: entries) }
    }

    func stageAll() async {
        await perform("Couldn’t Stage") { try $0.stageAll() }
    }

    func unstage(_ entries: [StatusEntry]) async {
        await perform("Couldn’t Unstage") { try $0.unstage(entries: entries) }
    }

    func unstageAll() async {
        await perform("Couldn’t Unstage") { try $0.unstageAll() }
    }

    /// Throws away unstaged changes; untracked files go to the trash. Ask first.
    func discard(_ entries: [StatusEntry]) async {
        await perform("Couldn’t Discard Changes") { try $0.discard(entries: entries) }
    }

    func apply(_ action: LineAction, to entry: StatusEntry, lines: LineSelection) async {
        switch action {
        case .stage:
            await perform("Couldn’t Stage Lines") { try $0.stageLines(entry: entry, selection: lines) }
        case .unstage:
            await perform("Couldn’t Unstage Lines") { try $0.unstageLines(entry: entry, selection: lines) }
        case .discard:
            await perform("Couldn’t Discard Lines") { try $0.discardLines(entry: entry, selection: lines) }
        }
    }

    // MARK: Committing

    func commit() async {
        guard canCommit else { return }
        let message = commitMessage
        let amend = amendCommit
        let newCommit = await perform("Couldn’t Commit", refreshHistory: true) {
            try $0.commit(message: message, amend: amend)
        }
        if let newCommit {
            commitMessage = ""
            amendCommit = false
            selectedCommitID = newCommit
        }
    }

    /// Puts the last commit's message in the editor when amending.
    func prefillAmendMessage() async {
        guard let repository, let head = info?.head.target else { return }
        if let details = try? await offMainActor({ try repository.commitDetails(id: head) }),
            commitMessage.isEmpty
        {
            commitMessage = details.message.trimmingCharacters(in: .whitespacesAndNewlines)
        }
    }

    // MARK: Branches

    func checkout(_ target: CheckoutTarget) async {
        await perform("Couldn’t Check Out", refreshHistory: true) { try $0.checkout(target: target) }
    }

    /// Returns whether the branch was created, so the sheet knows whether to close.
    func createBranch(name: String, start: String?, checkout: Bool) async -> Bool {
        await perform("Couldn’t Create Branch", refreshHistory: true) {
            try $0.createBranch(name: name, start: start, checkout: checkout)
        } != nil
    }

    func requestNewBranch(from commit: String? = nil, description: String? = nil) {
        newBranchRequest = NewBranchRequest(
            start: commit,
            startDescription: description ?? commit.map { String($0.prefix(8)) } ?? branchName ?? "HEAD")
    }

    // MARK: Remotes

    func fetch() async {
        await runRemote("Fetching", failure: "Fetch Failed") { repository, observer, cancel in
            try repository.fetch(remote: nil, observer: observer, cancel: cancel)
        }
    }

    func pull(_ mode: PullMode = .default) async {
        await runRemote("Pulling", failure: "Pull Failed") { repository, observer, cancel in
            try repository.pull(mode: mode, observer: observer, cancel: cancel)
        }
    }

    func push(force: Bool = false) async {
        await runRemote(force ? "Force Pushing" : "Pushing", failure: "Push Failed") {
            repository, observer, cancel in
            _ = try repository.push(force: force, observer: observer, cancel: cancel)
        }
    }

    func cancelOperation() {
        operationCancel?.cancel()
    }

    /// Writes git's commit-graph file, then reloads the (now faster) history.
    func writeCommitGraph() async {
        await runRemote("Optimizing History", failure: "Couldn’t Write Commit-Graph") {
            repository, _, cancel in
            try repository.writeCommitGraph(cancel: cancel)
        }
    }

    // MARK: Running operations

    /// Runs a quick local operation off the main actor, reports failures, and refreshes.
    /// Returns the operation's result, or `nil` if it failed.
    @discardableResult
    func perform<Result: Sendable>(
        _ failureTitle: String,
        refreshHistory: Bool = false,
        _ work: @escaping @Sendable (Repository) throws -> Result
    ) async -> Result? {
        guard let repository else { return nil }
        var result: Result?
        do {
            result = try await offMainActor { try work(repository) }
        } catch {
            alert = AlertMessage(title: failureTitle, message: describe(error))
        }
        await refreshRefsAndStatus()
        if refreshHistory {
            await loadHistory()
        }
        return result
    }

    /// Runs a long operation (network, maintenance) with progress, cancellation and credential
    /// prompts, then refreshes everything.
    private func runRemote(
        _ title: String,
        failure: String,
        _ work: @escaping @Sendable (Repository, RemoteObserver, CancelToken) throws -> Void
    ) async {
        guard let repository, operation == nil else { return }
        let cancel = CancelToken()
        operationCancel = cancel
        operation = OperationStatus(title: title)
        let relay = RemoteRelay { [weak self] progress in
            Task { @MainActor in
                guard let self, self.operation != nil else { return }
                self.operation?.detail = progress.phase
                self.operation?.fraction = progress.percent.map { Double($0) / 100 }
            }
        }
        do {
            try await offMainActor { try work(repository, relay, cancel) }
        } catch let error as BranchosaurusError where error.isCancellation {
            // The person cancelled.
        } catch {
            alert = AlertMessage(title: failure, message: describe(error))
        }
        operation = nil
        operationCancel = nil
        await refreshRefsAndStatus()
        await loadHistory()
    }
}
