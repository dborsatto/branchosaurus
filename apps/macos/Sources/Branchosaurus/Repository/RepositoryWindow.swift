// SPDX-License-Identifier: GPL-3.0-or-later

import AppKit
import BranchosaurusCore
import SwiftUI

struct RepositoryWindow: View {
    @State private var model: RepositoryModel
    @State private var confirmForcePush = false

    init(url: URL) {
        _model = State(initialValue: RepositoryModel(url: url))
    }

    var body: some View {
        content
            .navigationTitle(model.info?.name ?? model.url.lastPathComponent)
            .navigationSubtitle(model.branchName ?? "")
            .task { await model.open() }
            .onDisappear { model.close() }
            .onReceive(
                NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)
            ) { _ in
                Task { await model.refreshRefsAndStatus() }
            }
            .alert(
                model.alert?.title ?? "", isPresented: isPresented($model.alert), presenting: model.alert
            ) { _ in
                Button("OK", role: .cancel) {}
            } message: { alert in
                Text(alert.message)
            }
            .sheet(item: $model.newBranchRequest) { request in
                NewBranchSheet(model: model, request: request)
            }
            .confirmationDialog(
                "Force push “\(model.branchName ?? "")”?", isPresented: $confirmForcePush
            ) {
                Button("Force Push", role: .destructive) { Task { await model.push(force: true) } }
            } message: {
                Text(
                    "This replaces the branch on the remote with yours. It is refused if someone pushed commits you haven’t fetched."
                )
            }
    }

    @ViewBuilder
    private var content: some View {
        if let error = model.openError, model.repository == nil {
            ContentUnavailableView {
                Label("Can’t Open Repository", systemImage: "exclamationmark.triangle")
            } description: {
                Text(error)
            }
            .frame(minWidth: 520, minHeight: 360)
        } else {
            NavigationSplitView {
                SidebarView(model: model)
                    .navigationSplitViewColumnWidth(min: 180, ideal: 230, max: 360)
            } detail: {
                switch model.sidebarSelection {
                case .changes:
                    ChangesView(model: model)
                default:
                    HistoryView(model: model)
                }
            }
            .overlay(alignment: .bottom) {
                if let operation = model.operation {
                    OperationHUD(operation: operation) { model.cancelOperation() }
                        .padding(.bottom, 16)
                        .transition(.move(edge: .bottom).combined(with: .opacity))
                }
            }
            .animation(.default, value: model.operation?.title)
            .toolbar { toolbar }
        }
    }

    @ToolbarContentBuilder
    private var toolbar: some ToolbarContent {
        let busy = model.operation != nil
        let hasRemotes = !model.remotes.isEmpty
        let hasWorkdir = model.info?.workdir != nil
        ToolbarItemGroup(placement: .primaryAction) {
            Button {
                Task { await model.fetch() }
            } label: {
                Label("Fetch", systemImage: "arrow.down.circle")
            }
            .help("Fetch from all remotes (⇧⌘F)")
            .keyboardShortcut("f", modifiers: [.command, .shift])
            .disabled(busy || !hasRemotes)

            Menu {
                Button("Pull (Fast-Forward Only)") { Task { await model.pull(.fastForwardOnly) } }
                Button("Pull with Merge") { Task { await model.pull(.merge) } }
                Button("Pull with Rebase") { Task { await model.pull(.rebase) } }
            } label: {
                Label("Pull", systemImage: "arrow.down.to.line.circle")
            } primaryAction: {
                Task { await model.pull() }
            }
            .help("Pull the current branch’s upstream (⇧⌘P)")
            .keyboardShortcut("p", modifiers: [.command, .shift])
            .disabled(busy || model.info?.head.isDetached ?? true || !hasRemotes || !hasWorkdir)

            Menu {
                Button("Force Push (with Lease)…") { confirmForcePush = true }
            } label: {
                Label("Push", systemImage: "arrow.up.circle")
            } primaryAction: {
                Task { await model.push() }
            }
            .help("Push the current branch (⇧⌘U)")
            .keyboardShortcut("u", modifiers: [.command, .shift])
            .disabled(busy || model.info?.head.isDetached ?? true || !hasRemotes)

            Button {
                model.requestNewBranch()
            } label: {
                Label("New Branch", systemImage: "arrow.triangle.branch")
            }
            .help("Create a branch at HEAD (⇧⌘B)")
            .keyboardShortcut("b", modifiers: [.command, .shift])
            .disabled(busy || model.info?.head.target == nil)

            Button {
                Task { await model.reload() }
            } label: {
                Label("Refresh", systemImage: "arrow.clockwise")
            }
            .keyboardShortcut("r")
            .help("Reload branches, history and changes (⌘R)")
        }
    }
}

extension Head {
    var isDetached: Bool {
        if case .detached = self { return true }
        return false
    }
}

/// Progress of a fetch, pull or push, floating over the window.
private struct OperationHUD: View {
    let operation: OperationStatus
    let cancel: () -> Void

    var body: some View {
        HStack(spacing: 12) {
            if let fraction = operation.fraction {
                ProgressView(value: fraction)
                    .progressViewStyle(.circular)
                    .controlSize(.small)
            } else {
                ProgressView().controlSize(.small)
            }
            VStack(alignment: .leading, spacing: 1) {
                Text(operation.title + "…").font(.callout.weight(.medium))
                if let detail = operation.detail {
                    Text(detail + (operation.fraction.map { " \(Int($0 * 100))%" } ?? ""))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .monospacedDigit()
                }
            }
            .frame(minWidth: 160, alignment: .leading)
            Button("Cancel", action: cancel)
                .controlSize(.small)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        .background(.regularMaterial, in: Capsule())
        .shadow(radius: 8, y: 2)
        .accessibilityElement(children: .combine)
    }
}

/// Names and creates a branch.
struct NewBranchSheet: View {
    let model: RepositoryModel
    let request: NewBranchRequest

    @Environment(\.dismiss) private var dismiss
    @State private var name = ""
    @State private var checkout = true
    @State private var creating = false

    private var trimmed: String { name.trimmingCharacters(in: .whitespaces) }
    private var isValid: Bool { isValidBranchName(name: trimmed) }
    private var exists: Bool { model.refs?.localBranches.contains { $0.name == trimmed } ?? false }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("New Branch").font(.headline)
            Text("Starts at \(request.startDescription).")
                .foregroundStyle(.secondary)
            TextField("Branch name", text: $name)
                .textFieldStyle(.roundedBorder)
                .onSubmit(create)
            if !trimmed.isEmpty && !isValid {
                Text("Branch names can’t contain spaces, “..”, “~”, “^”, “:”, “?”, “*”, “[” or end in “.lock”.")
                    .font(.caption)
                    .foregroundStyle(.red)
            } else if exists {
                Text("A branch named “\(trimmed)” already exists.")
                    .font(.caption)
                    .foregroundStyle(.red)
            }
            Toggle("Check out the new branch", isOn: $checkout)
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Create Branch", action: create)
                    .keyboardShortcut(.defaultAction)
                    .disabled(!isValid || exists || creating)
            }
        }
        .padding(20)
        .frame(width: 380)
    }

    private func create() {
        guard isValid, !exists, !creating else { return }
        creating = true
        Task {
            if await model.createBranch(name: trimmed, start: request.start, checkout: checkout) {
                dismiss()
            }
            creating = false
        }
    }
}
