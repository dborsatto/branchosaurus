// SPDX-License-Identifier: GPL-3.0-or-later

import BranchosaurusCore
import SwiftUI

/// The selected commit: its message and metadata, the files it changed, and the diff of one file.
struct CommitDetailView: View {
    let model: RepositoryModel
    let commitID: String?

    @State private var details: CommitDetails?
    @State private var detailsError: String?
    @State private var selectedFile: FileChange.ID?
    @State private var diff: DiffState = .idle

    var body: some View {
        if commitID == nil {
            ContentUnavailableView("No Commit Selected", systemImage: "point.topleft.down.to.point.bottomright.curvepath")
        } else {
            HSplitView {
                VStack(alignment: .leading, spacing: 0) {
                    if let details {
                        CommitHeader(details: details)
                        Divider()
                        List(details.changes, selection: $selectedFile) { change in
                            FileChangeRow(change: change)
                        }
                        .listStyle(.inset)
                    } else if let detailsError {
                        ContentUnavailableView(
                            "Can’t Read Commit", systemImage: "exclamationmark.triangle",
                            description: Text(detailsError))
                    } else {
                        ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
                    }
                }
                .frame(minWidth: 260, idealWidth: 340, maxWidth: 520)

                DiffPane(state: diff)
                    .frame(minWidth: 320)
            }
            .task(id: commitID) { await loadDetails() }
            .task(id: selectedFile) { await loadDiff() }
        }
    }

    private func loadDetails() async {
        guard let commitID, let repository = model.repository else { return }
        details = nil
        detailsError = nil
        diff = .idle
        do {
            let loaded = try await offMainActor { try repository.commitDetails(id: commitID) }
            guard !Task.isCancelled else { return }
            details = loaded
            selectedFile = loaded.changes.first?.id
        } catch {
            detailsError = (error as? BranchosaurusError)?.message ?? error.localizedDescription
        }
    }

    private func loadDiff() async {
        guard let repository = model.repository,
            let change = details?.changes.first(where: { $0.id == selectedFile })
        else {
            diff = .idle
            return
        }
        diff = .loading
        do {
            let loaded = try await offMainActor { try repository.commitFileDiff(change: change) }
            guard !Task.isCancelled else { return }
            diff = .loaded(loaded)
        } catch {
            diff = .failed((error as? BranchosaurusError)?.message ?? error.localizedDescription)
        }
    }
}

private struct CommitHeader: View {
    let details: CommitDetails

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(details.summary)
                .font(.headline)
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
            if !details.body.isEmpty {
                ScrollView {
                    Text(details.body)
                        .font(.callout)
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .frame(maxHeight: 120)
            }
            Grid(alignment: .leading, horizontalSpacing: 8, verticalSpacing: 2) {
                GridRow {
                    Text("Author").foregroundStyle(.secondary)
                    Text("\(details.author.name) <\(details.author.email)>")
                        .lineLimit(1).truncationMode(.tail)
                }
                GridRow {
                    Text("Date").foregroundStyle(.secondary)
                    Text(details.author.date.formatted(date: .long, time: .shortened))
                }
                if details.committer.name != details.author.name || details.committer.time != details.author.time {
                    GridRow {
                        Text("Committer").foregroundStyle(.secondary)
                        Text(
                            "\(details.committer.name), \(details.committer.date.formatted(date: .abbreviated, time: .shortened))"
                        )
                        .lineLimit(1)
                    }
                }
                GridRow {
                    Text("Commit").foregroundStyle(.secondary)
                    Text(details.id).font(.caption.monospaced()).textSelection(.enabled).lineLimit(1)
                }
                if !details.parents.isEmpty {
                    GridRow {
                        Text(details.parents.count > 1 ? "Parents" : "Parent").foregroundStyle(.secondary)
                        Text(details.parents.map { String($0.prefix(8)) }.joined(separator: ", "))
                            .font(.caption.monospaced())
                            .textSelection(.enabled)
                    }
                }
            }
            .font(.callout)
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

struct FileChangeRow: View {
    let change: FileChange

    var body: some View {
        HStack(spacing: 6) {
            ChangeKindBadge(kind: change.kind)
            VStack(alignment: .leading, spacing: 0) {
                Text(fileName(change.path))
                    .lineLimit(1)
                    .truncationMode(.middle)
                if let directory = directory(change.path, oldPath: change.oldPath) {
                    Text(directory)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
            }
            Spacer(minLength: 4)
            if let stats = change.stats {
                LineStatsView(stats: stats)
            }
        }
        .help(change.path.display)
    }
}

struct LineStatsView: View {
    let stats: LineStats

    var body: some View {
        HStack(spacing: 4) {
            if stats.additions > 0 {
                Text("+\(stats.additions)").foregroundStyle(.green)
            }
            if stats.deletions > 0 {
                Text("−\(stats.deletions)").foregroundStyle(.red)
            }
        }
        .font(.caption.monospacedDigit())
    }
}

struct ChangeKindBadge: View {
    let kind: ChangeKind

    var body: some View {
        Image(systemName: kind.symbolName)
            .foregroundStyle(kind.color)
            .help(kind.title)
            .accessibilityLabel(kind.title)
    }
}

extension ChangeKind {
    var title: String {
        switch self {
        case .added: "Added"
        case .deleted: "Deleted"
        case .modified: "Modified"
        case .renamed: "Renamed"
        case .copied: "Copied"
        case .typeChanged: "Type changed"
        case .untracked: "Untracked"
        }
    }

    var symbolName: String {
        switch self {
        case .added: "plus.circle.fill"
        case .deleted: "minus.circle.fill"
        case .modified: "pencil.circle.fill"
        case .renamed: "arrow.right.circle.fill"
        case .copied: "doc.on.doc.fill"
        case .typeChanged: "arrow.triangle.2.circlepath.circle.fill"
        case .untracked: "questionmark.circle.fill"
        }
    }

    var color: Color {
        switch self {
        case .added, .untracked: .green
        case .deleted: .red
        case .modified, .typeChanged: .orange
        case .renamed, .copied: .blue
        }
    }
}

func fileName(_ path: RepoPath) -> String {
    path.display.split(separator: "/").last.map(String.init) ?? path.display
}

/// The containing directory, or for renames, where the file came from.
func directory(_ path: RepoPath, oldPath: RepoPath? = nil) -> String? {
    if let oldPath {
        return "from \(oldPath.display)"
    }
    let components = path.display.split(separator: "/")
    guard components.count > 1 else { return nil }
    return components.dropLast().joined(separator: "/")
}
