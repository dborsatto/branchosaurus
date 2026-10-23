// SPDX-License-Identifier: GPL-3.0-or-later

import AppKit
import BranchosaurusCore
import SwiftUI

/// Commit list on top, the selected commit's files and diff below.
struct HistoryView: View {
    @Bindable var model: RepositoryModel
    @State private var dismissedBanner = false

    /// Big histories without git's commit-graph file load several times slower than they could.
    private func suggestsCommitGraph(_ history: History) -> Bool {
        let stats = history.stats()
        return !dismissedBanner && stats.commitsFromCommitGraph == 0 && stats.commits > 20_000
    }

    private func run(_ command: HistoryCommand, on id: String) {
        switch command {
        case .checkout:
            Task { await model.checkout(.commit(id: id)) }
        case .newBranch:
            model.requestNewBranch(from: id)
        case .copyID:
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(id, forType: .string)
        }
    }

    var body: some View {
        if let history = model.history {
            if history.rowCount() == 0 {
                ContentUnavailableView(
                    "No Commits Yet", systemImage: "circle.dashed",
                    description: Text("Commits will appear here once you make the first one."))
            } else {
                VSplitView {
                    VStack(spacing: 0) {
                        if suggestsCommitGraph(history) {
                            CommitGraphBanner(model: model) { dismissedBanner = true }
                            Divider()
                        }
                        HistoryTableView(history: history, selection: $model.selectedCommitID) {
                            command, id in
                            run(command, on: id)
                        }
                    }
                    .frame(minHeight: 140, idealHeight: 360)
                    CommitDetailView(model: model, commitID: model.selectedCommitID)
                        .frame(minHeight: 200)
                }
            }
        } else if let error = model.historyError {
            ContentUnavailableView {
                Label("Can’t Load History", systemImage: "exclamationmark.triangle")
            } description: {
                Text(error)
            } actions: {
                Button("Try Again") { Task { await model.loadHistory() } }
            }
        } else {
            VStack(spacing: 8) {
                ProgressView()
                if let progress = model.historyProgress {
                    Text(
                        progress.phase == .indexing
                            ? "Reading \(progress.commits.formatted()) commits…"
                            : "Laying out \(progress.commits.formatted()) commits…"
                    )
                    .foregroundStyle(.secondary)
                    .monospacedDigit()
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }
}

/// Offers to write git's commit-graph file.
private struct CommitGraphBanner: View {
    let model: RepositoryModel
    let dismiss: () -> Void

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: "bolt.circle.fill")
                .foregroundStyle(.tint)
                .font(.title3)
            Text("History would load several times faster with git’s commit-graph file.")
                .font(.callout)
            Spacer()
            Button("Not Now", action: dismiss)
            Button("Create It") { Task { await model.writeCommitGraph() } }
                .buttonStyle(.borderedProminent)
                .disabled(model.operation != nil)
        }
        .controlSize(.small)
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(.bar)
    }
}
