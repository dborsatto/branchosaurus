// SPDX-License-Identifier: GPL-3.0-or-later

import AppKit
import BranchosaurusCore
import SwiftUI

/// The working copy: unstaged and staged files, the diff of the selected one with hunk and line
/// actions, and the commit message.
struct ChangesView: View {
    @Bindable var model: RepositoryModel

    @State private var selection: Set<ChangeSelection> = []
    @State private var diff: DiffState = .idle
    @State private var selectedLines = LineSelection(removed: [], added: [])
    @State private var pendingDiscard: PendingDiscard?

    var body: some View {
        if let status = model.status {
            HSplitView {
                VStack(spacing: 0) {
                    fileList(status)
                    Divider()
                    CommitComposer(model: model)
                }
                .frame(minWidth: 280, idealWidth: 340, maxWidth: 520)

                diffColumn(status)
                    .frame(minWidth: 320)
            }
            // Reload the diff when the selection changes, and when the status is refreshed
            // because the file may have changed on disk.
            .task(id: DiffKey(selection: selection, status: status)) {
                let stillThere = selection.filter { status.entry(for: $0) != nil }
                if stillThere != selection {
                    selection = stillThere
                }
                if selection.isEmpty, let first = status.firstSelection {
                    selection = [first]
                    return
                }
                await loadDiff(status: status)
            }
            .alert(
                pendingDiscard?.title ?? "", isPresented: isPresented($pendingDiscard),
                presenting: pendingDiscard
            ) { discard in
                Button("Discard", role: .destructive) {
                    Task { await discard.run(model) }
                }
                Button("Cancel", role: .cancel) {}
            } message: { discard in
                Text(discard.message)
            }
        } else if model.info != nil && model.info?.workdir == nil {
            ContentUnavailableView(
                "Bare Repository", systemImage: "shippingbox",
                description: Text("This repository has no working copy."))
        } else {
            ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    // MARK: File list

    private func fileList(_ status: WorkingCopyStatus) -> some View {
        List(selection: $selection) {
            if !status.conflicted.isEmpty {
                Section("Conflicts (\(status.conflicted.count))") {
                    ForEach(status.conflicted, id: \.bytes) { path in
                        Label {
                            Text(path.display).lineLimit(1).truncationMode(.middle)
                        } icon: {
                            Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.yellow)
                        }
                        .selectionDisabled()
                    }
                }
            }
            Section {
                ForEach(status.unstaged) { entry in
                    StatusRow(entry: entry).tag(ChangeSelection.unstaged(entry.id))
                }
            } header: {
                SectionHeader(
                    title: "Unstaged", count: status.unstaged.count, actionTitle: "Stage All",
                    disabled: status.unstaged.isEmpty
                ) {
                    Task { await model.stageAll() }
                }
            }
            Section {
                ForEach(status.staged) { entry in
                    StatusRow(entry: entry).tag(ChangeSelection.staged(entry.id))
                }
            } header: {
                SectionHeader(
                    title: "Staged", count: status.staged.count, actionTitle: "Unstage All",
                    disabled: status.staged.isEmpty
                ) {
                    Task { await model.unstageAll() }
                }
            }
        }
        .listStyle(.inset)
        .contextMenu(forSelectionType: ChangeSelection.self) { items in
            fileMenu(items, status: status)
        } primaryAction: { items in
            // Double-click or Return moves files to the other list.
            Task { await toggle(items, status: status) }
        }
        .overlay {
            if status.unstaged.isEmpty && status.staged.isEmpty && status.conflicted.isEmpty {
                ContentUnavailableView(
                    "No Changes", systemImage: "checkmark.circle",
                    description: Text("The working copy matches the last commit."))
            }
        }
    }

    @ViewBuilder
    private func fileMenu(_ items: Set<ChangeSelection>, status: WorkingCopyStatus) -> some View {
        let unstaged = status.entries(items, staged: false)
        let staged = status.entries(items, staged: true)
        if !unstaged.isEmpty {
            Button(unstaged.count == 1 ? "Stage" : "Stage \(unstaged.count) Files") {
                Task { await model.stage(unstaged) }
            }
            Button(unstaged.count == 1 ? "Discard Changes…" : "Discard Changes to \(unstaged.count) Files…") {
                pendingDiscard = .files(unstaged)
            }
        }
        if !staged.isEmpty {
            Button(staged.count == 1 ? "Unstage" : "Unstage \(staged.count) Files") {
                Task { await model.unstage(staged) }
            }
        }
        if let workdir = model.info?.workdir {
            let entries = unstaged + staged
            Divider()
            Button("Show in Finder") {
                let urls = entries
                    .map { URL(fileURLWithPath: workdir).appendingPathComponent($0.path.display) }
                    .filter { FileManager.default.fileExists(atPath: $0.path) }
                NSWorkspace.shared.activateFileViewerSelecting(urls)
            }
            Button("Copy Path") {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(
                    entries.map(\.path.display).joined(separator: "\n"), forType: .string)
            }
        }
    }

    private func toggle(_ items: Set<ChangeSelection>, status: WorkingCopyStatus) async {
        let unstaged = status.entries(items, staged: false)
        let staged = status.entries(items, staged: true)
        if !unstaged.isEmpty { await model.stage(unstaged) }
        if !staged.isEmpty { await model.unstage(staged) }
    }

    // MARK: Diff

    @ViewBuilder
    private func diffColumn(_ status: WorkingCopyStatus) -> some View {
        if selection.count > 1 {
            ContentUnavailableView(
                "\(selection.count) Files Selected", systemImage: "doc.on.doc",
                description: Text("Double-click to move them between Unstaged and Staged."))
        } else if let item = selection.first, let entry = status.entry(for: item) {
            VStack(spacing: 0) {
                DiffHeader(
                    entry: entry, isStaged: item.isStaged, selectedLines: selectedLines,
                    perform: { action in Task { await fileAction(action, entry: entry) } },
                    performLines: { action in lineAction(action, entry: entry, lines: selectedLines) })
                Divider()
                DiffPane(
                    state: diff,
                    actions: DiffActions(mode: item.isStaged ? .staged : .unstaged) { action, lines in
                        lineAction(action, entry: entry, lines: lines)
                    },
                    selectedLines: $selectedLines)
            }
        } else {
            DiffPane(state: .idle)
        }
    }

    private func fileAction(_ action: LineAction, entry: StatusEntry) async {
        switch action {
        case .stage: await model.stage([entry])
        case .unstage: await model.unstage([entry])
        case .discard: pendingDiscard = .files([entry])
        }
    }

    private func lineAction(_ action: LineAction, entry: StatusEntry, lines: LineSelection) {
        guard !lines.removed.isEmpty || !lines.added.isEmpty else { return }
        if action == .discard {
            pendingDiscard = .lines(entry, lines)
        } else {
            Task { await model.apply(action, to: entry, lines: lines) }
        }
    }

    private func loadDiff(status: WorkingCopyStatus) async {
        guard let repository = model.repository, selection.count == 1, let item = selection.first,
            let entry = status.entry(for: item)
        else {
            diff = .idle
            return
        }
        if case .idle = diff { diff = .loading }
        do {
            let loaded = try await offMainActor {
                item.isStaged
                    ? try repository.stagedDiff(entry: entry) : try repository.unstagedDiff(entry: entry)
            }
            guard !Task.isCancelled else { return }
            diff = .loaded(loaded)
        } catch {
            diff = .failed(model.describe(error))
        }
        selectedLines = LineSelection(removed: [], added: [])
    }
}

/// A file in one of the two lists.
enum ChangeSelection: Hashable, Sendable {
    case unstaged(Data)
    case staged(Data)

    var isStaged: Bool {
        if case .staged = self { return true }
        return false
    }
}

extension WorkingCopyStatus {
    func entry(for item: ChangeSelection) -> StatusEntry? {
        switch item {
        case let .unstaged(id): unstaged.first { $0.id == id }
        case let .staged(id): staged.first { $0.id == id }
        }
    }

    func entries(_ items: Set<ChangeSelection>, staged: Bool) -> [StatusEntry] {
        items.filter { $0.isStaged == staged }.compactMap(entry(for:))
    }

    var firstSelection: ChangeSelection? {
        if let entry = unstaged.first { return .unstaged(entry.id) }
        if let entry = staged.first { return .staged(entry.id) }
        return nil
    }
}

private struct DiffKey: Hashable {
    let selection: Set<ChangeSelection>
    let status: WorkingCopyStatus
}

/// A discard waiting for confirmation: it can't be undone (untracked files go to the trash).
private enum PendingDiscard: Sendable {
    case files([StatusEntry])
    case lines(StatusEntry, LineSelection)

    var title: String {
        switch self {
        case let .files(entries) where entries.count == 1:
            "Discard changes to “\(fileName(entries[0].path))”?"
        case let .files(entries): "Discard changes to \(entries.count) files?"
        case .lines: "Discard the selected lines?"
        }
    }

    var message: String {
        switch self {
        case let .files(entries) where entries.contains { $0.kind == .untracked }:
            "Untracked files are moved to the Trash. Other changes can’t be recovered."
        default: "This can’t be undone."
        }
    }

    @MainActor func run(_ model: RepositoryModel) async {
        switch self {
        case let .files(entries): await model.discard(entries)
        case let .lines(entry, lines): await model.apply(.discard, to: entry, lines: lines)
        }
    }
}

/// A binding that is true while `value` is set, and clears it when dismissed.
func isPresented<Value: Sendable>(_ value: Binding<Value?>) -> Binding<Bool> {
    Binding(get: { value.wrappedValue != nil }, set: { if !$0 { value.wrappedValue = nil } })
}

private struct SectionHeader: View {
    let title: String
    let count: Int
    let actionTitle: String
    let disabled: Bool
    let action: () -> Void

    var body: some View {
        HStack {
            Text("\(title) (\(count))")
            Spacer()
            Button(actionTitle, action: action)
                .buttonStyle(.borderless)
                .controlSize(.small)
                .disabled(disabled)
        }
    }
}

/// The selected file's name and the actions for the whole file or the selected lines.
private struct DiffHeader: View {
    let entry: StatusEntry
    let isStaged: Bool
    let selectedLines: LineSelection
    let perform: (LineAction) -> Void
    let performLines: (LineAction) -> Void

    var body: some View {
        HStack(spacing: 8) {
            ChangeKindBadge(kind: entry.kind)
            Text(entry.path.display)
                .lineLimit(1)
                .truncationMode(.head)
                .help(entry.path.display)
            Spacer(minLength: 12)
            let lineCount = selectedLines.removed.count + selectedLines.added.count
            if lineCount > 0 {
                if isStaged {
                    Button("Unstage \(lineCount) Lines") { performLines(.unstage) }
                } else {
                    Button("Discard \(lineCount) Lines…") { performLines(.discard) }
                    Button("Stage \(lineCount) Lines") { performLines(.stage) }
                        .keyboardShortcut("s", modifiers: [.command, .shift])
                }
            } else if isStaged {
                Button("Unstage File") { perform(.unstage) }
            } else {
                Button("Discard File…") { perform(.discard) }
                Button("Stage File") { perform(.stage) }
                    .keyboardShortcut("s", modifiers: [.command, .shift])
            }
        }
        .controlSize(.small)
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
    }
}

private struct StatusRow: View {
    let entry: StatusEntry

    var body: some View {
        HStack(spacing: 6) {
            ChangeKindBadge(kind: entry.kind)
            VStack(alignment: .leading, spacing: 0) {
                Text(fileName(entry.path))
                    .lineLimit(1)
                    .truncationMode(.middle)
                if let directory = directory(entry.path, oldPath: entry.oldPath) {
                    Text(directory)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
            }
        }
        .help(entry.path.display)
    }
}
