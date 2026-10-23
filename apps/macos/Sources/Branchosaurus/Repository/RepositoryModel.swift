// SPDX-License-Identifier: GPL-3.0-or-later

import BranchosaurusCore
import Foundation
import Observation

enum SidebarItem: Hashable {
    case changes
    case history
    case localBranch(fullName: String)
    case remoteBranch(fullName: String)
    case tag(fullName: String)
}

/// A long-running remote operation, shown with progress and a Cancel button.
struct OperationStatus: Equatable {
    var title: String
    var detail: String?
    /// 0…1, when git reports a percentage.
    var fraction: Double?
}

/// An error to show in an alert.
struct AlertMessage: Identifiable, Equatable, Sendable {
    let id = UUID()
    let title: String
    let message: String
}

/// A request to show the New Branch sheet.
struct NewBranchRequest: Identifiable {
    let id = UUID()
    /// Commit to start from, or `nil` for HEAD.
    let start: String?
    /// What the branch starts from, for the sheet's text ("main", "a1b2c3d4").
    let startDescription: String
}

/// Everything one repository window shows, and the actions it offers. Core calls block, so they
/// run on background tasks and their results are applied here, on the main actor.
@MainActor @Observable
final class RepositoryModel {
    let url: URL

    private(set) var repository: Repository?
    private(set) var info: RepositoryInfo?
    private(set) var refs: Refs?
    /// Names of the configured remotes.
    private(set) var remotes: [String] = []
    private(set) var status: WorkingCopyStatus?
    private(set) var history: History?
    private(set) var historyProgress: HistoryProgress?
    private(set) var openError: String?
    private(set) var historyError: String?

    var sidebarSelection: SidebarItem? = RepositoryModel.initialSidebarSelection
    /// The commit selected in the history list.
    var selectedCommitID: String?

    /// The commit message being written, kept while switching between views.
    var commitMessage = ""
    var amendCommit = false {
        didSet {
            if amendCommit && !oldValue && commitMessage.isEmpty {
                Task { await prefillAmendMessage() }
            }
        }
    }

    /// The remote operation in progress, if any.
    var operation: OperationStatus?
    var alert: AlertMessage?
    var newBranchRequest: NewBranchRequest?

    @ObservationIgnored var operationCancel: CancelToken?
    @ObservationIgnored private var historyCancel: CancelToken?
    @ObservationIgnored private var historyGeneration = 0
    @ObservationIgnored private var watcher: Watcher?
    @ObservationIgnored private var statusRefresh = Coalescer()
    @ObservationIgnored private var historyRefresh = Coalescer()

    init(url: URL) {
        self.url = url
    }

    private static var initialSidebarSelection: SidebarItem {
        #if DEBUG
            if ProcessInfo.processInfo.environment["BRANCHOSAURUS_SIDEBAR"] == "changes" {
                return .changes
            }
        #endif
        return .history
    }

    var branchName: String? {
        switch info?.head {
        case let .branch(name, _): name
        case let .detached(target): String(target.prefix(8))
        case nil: nil
        }
    }

    var changeCount: Int {
        guard let status else { return 0 }
        return status.staged.count + status.unstaged.count + status.conflicted.count
    }

    var canCommit: Bool {
        let hasMessage = !commitMessage.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        let hasChanges = !(status?.staged.isEmpty ?? true)
        return hasMessage && (hasChanges || amendCommit) && operation == nil
    }

    // MARK: Loading

    func open() async {
        openError = nil
        let path = url.path
        do {
            repository = try await offMainActor { try Repository.discover(path: path) }
            await reload()
            startWatching()
        } catch {
            openError = describe(error)
        }
    }

    func reload() async {
        await refreshRefsAndStatus()
        await loadHistory()
    }

    /// Branches, HEAD and the working copy. Cheap enough to run whenever something changes.
    func refreshRefsAndStatus() async {
        guard let repository else { return }
        await statusRefresh.run { [weak self] in
            do {
                let (info, refs, remotes, status) = try await offMainActor {
                    let info = try repository.info()
                    let status = info.workdir == nil ? nil : try repository.status()
                    return (info, try repository.refs(), try repository.remotes(), status)
                }
                guard let self else { return }
                self.info = info
                self.refs = refs
                self.remotes = remotes
                self.status = status
            } catch {
                self?.openError = self?.describe(error)
            }
        }
    }

    func loadHistory() async {
        guard let repository else { return }
        await historyRefresh.run { [weak self] in
            await self?.loadHistory(from: repository)
        }
    }

    private func loadHistory(from repository: Repository) async {
        historyCancel?.cancel()
        let cancel = CancelToken()
        historyCancel = cancel
        historyGeneration += 1
        let generation = historyGeneration
        historyError = nil

        let relay = ProgressRelay { [weak self] progress in
            Task { @MainActor in
                guard let self, self.historyGeneration == generation else { return }
                self.historyProgress = progress
            }
        }
        do {
            let history = try await offMainActor {
                try repository.loadHistory(observer: relay, cancel: cancel)
            }
            guard generation == historyGeneration else { return }
            self.history = history
            historyProgress = nil
            if selectedCommitID == nil {
                selectedCommitID = info?.head.target ?? (try? history.rows(start: 0, count: 1))?.first?.id
            }
        } catch let error as BranchosaurusError where error.isCancellation {
            // A newer load replaced this one.
        } catch {
            historyError = describe(error)
        }
    }

    /// Selects the commit a ref points to.
    func reveal(_ item: SidebarItem) {
        if let target = target(of: item) {
            selectedCommitID = target
        }
    }

    func target(of item: SidebarItem) -> String? {
        guard let refs else { return nil }
        switch item {
        case let .localBranch(fullName):
            return refs.localBranches.first { $0.fullName == fullName }?.target
        case let .remoteBranch(fullName):
            return refs.remoteBranches.first { $0.fullName == fullName }?.target
        case let .tag(fullName):
            return refs.tags.first { $0.fullName == fullName }?.target
        case .changes, .history:
            return nil
        }
    }

    // MARK: Watching

    private func startWatching() {
        guard let repository, watcher == nil else { return }
        let relay = WatchRelay { [weak self] event in
            Task { @MainActor in await self?.repositoryChanged(event) }
        }
        watcher = try? repository.watch(observer: relay)
    }

    private func repositoryChanged(_ event: WatchEvent) async {
        // Our own operations refresh when they finish.
        guard operation == nil else { return }
        await refreshRefsAndStatus()
        if event.refs {
            await loadHistory()
        }
    }

    func close() {
        historyCancel?.cancel()
        operationCancel?.cancel()
        watcher?.stop()
        watcher = nil
    }

    func describe(_ error: Error) -> String {
        (error as? BranchosaurusError)?.message ?? error.localizedDescription
    }
}

/// Runs a refresh, and if more requests arrive while it runs, runs it once more afterwards instead
/// of once per request.
@MainActor
final class Coalescer {
    private var running = false
    private var again = false

    func run(_ work: @escaping @MainActor () async -> Void) async {
        if running {
            again = true
            return
        }
        running = true
        repeat {
            again = false
            await work()
        } while again
        running = false
    }
}

extension Head {
    /// The commit HEAD points to, if there is one.
    var target: String? {
        switch self {
        case let .branch(_, target): target
        case let .detached(target): target
        }
    }
}

/// Runs blocking core work off the main actor.
func offMainActor<T: Sendable>(_ work: @escaping @Sendable () throws -> T) async throws -> T {
    try await Task.detached(priority: .userInitiated, operation: work).value
}
