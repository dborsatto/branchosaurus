// SPDX-License-Identifier: GPL-3.0-or-later

import AppKit
import BranchosaurusCore
import SwiftUI

struct SidebarView: View {
    @Bindable var model: RepositoryModel

    var body: some View {
        List(selection: $model.sidebarSelection) {
            Section("Workspace") {
                if model.info?.workdir != nil {
                    Label("Changes", systemImage: "square.and.pencil")
                        .badge(model.changeCount)
                        .tag(SidebarItem.changes)
                }
                Label("History", systemImage: "clock.arrow.circlepath")
                    .tag(SidebarItem.history)
            }

            if let refs = model.refs {
                if !refs.localBranches.isEmpty {
                    Section("Branches") {
                        ForEach(refs.localBranches, id: \.fullName) { branch in
                            Label {
                                Text(branch.name)
                                    .fontWeight(branch.isHead ? .semibold : .regular)
                            } icon: {
                                Image(systemName: branch.isHead ? "checkmark.circle.fill" : "arrow.triangle.branch")
                            }
                            .help(branch.upstream.map { "Tracks \($0)" } ?? branch.name)
                            .tag(SidebarItem.localBranch(fullName: branch.fullName))
                        }
                    }
                }

                if !refs.remoteBranches.isEmpty {
                    Section("Remotes") {
                        ForEach(remotes(refs), id: \.name) { remote in
                            DisclosureGroup {
                                ForEach(remote.branches, id: \.fullName) { branch in
                                    Label(
                                        String(branch.name.dropFirst(remote.name.count + 1)),
                                        systemImage: "arrow.triangle.branch"
                                    )
                                    .tag(SidebarItem.remoteBranch(fullName: branch.fullName))
                                }
                            } label: {
                                Label(remote.name, systemImage: "network")
                            }
                        }
                    }
                }

                if !refs.tags.isEmpty {
                    Section("Tags") {
                        ForEach(refs.tags.reversed(), id: \.fullName) { tag in
                            Label(tag.name, systemImage: "tag")
                                .tag(SidebarItem.tag(fullName: tag.fullName))
                        }
                    }
                }
            }
        }
        .listStyle(.sidebar)
        .onChange(of: model.sidebarSelection) { _, item in
            if let item {
                model.reveal(item)
            }
        }
        .contextMenu(forSelectionType: SidebarItem.self) { items in
            if let item = items.first, items.count == 1 {
                refMenu(item)
            }
        } primaryAction: { items in
            // Double-click checks a branch out.
            if let item = items.first, let target = checkoutTarget(item) {
                Task { await model.checkout(target) }
            }
        }
    }

    @ViewBuilder
    private func refMenu(_ item: SidebarItem) -> some View {
        if let target = checkoutTarget(item) {
            Button(checkoutTitle(item)) { Task { await model.checkout(target) } }
                .disabled(isHead(item))
        }
        if let commit = model.target(of: item) {
            Button("New Branch from Here…") {
                model.requestNewBranch(from: commit, description: displayName(item))
            }
            Divider()
            Button("Copy Commit ID") {
                NSPasteboard.general.clearContents()
                NSPasteboard.general.setString(commit, forType: .string)
            }
        }
    }

    private func checkoutTarget(_ item: SidebarItem) -> CheckoutTarget? {
        guard let refs = model.refs else { return nil }
        switch item {
        case let .localBranch(fullName):
            return refs.localBranches.first { $0.fullName == fullName }
                .map { .localBranch(name: $0.name) }
        case let .remoteBranch(fullName):
            return refs.remoteBranches.first { $0.fullName == fullName }
                .map { .remoteBranch(name: $0.name) }
        case let .tag(fullName):
            return refs.tags.first { $0.fullName == fullName }.map { .commit(id: $0.target) }
        case .changes, .history:
            return nil
        }
    }

    private func checkoutTitle(_ item: SidebarItem) -> String {
        switch item {
        case .remoteBranch: "Check Out as Local Branch"
        case .tag: "Check Out (Detached HEAD)"
        default: "Check Out"
        }
    }

    private func isHead(_ item: SidebarItem) -> Bool {
        guard case let .localBranch(fullName) = item else { return false }
        return model.refs?.localBranches.first { $0.fullName == fullName }?.isHead ?? false
    }

    private func displayName(_ item: SidebarItem) -> String? {
        switch item {
        case let .localBranch(fullName), let .remoteBranch(fullName), let .tag(fullName):
            fullName.split(separator: "/", maxSplits: 2).dropFirst(2).first.map(String.init) ?? fullName
        case .changes, .history:
            nil
        }
    }

    private struct Remote {
        let name: String
        let branches: [RemoteBranch]
    }

    private func remotes(_ refs: Refs) -> [Remote] {
        Dictionary(grouping: refs.remoteBranches, by: \.remote)
            .map { Remote(name: $0.key, branches: $0.value) }
            .sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
    }
}
