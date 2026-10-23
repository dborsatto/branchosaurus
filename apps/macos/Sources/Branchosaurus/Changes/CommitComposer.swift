// SPDX-License-Identifier: GPL-3.0-or-later

import SwiftUI

/// The commit message editor, the Amend switch and the Commit button.
struct CommitComposer: View {
    @Bindable var model: RepositoryModel
    @FocusState private var editing: Bool

    /// Git tools show summaries longer than this truncated.
    private static let summaryLimit = 72

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ZStack(alignment: .topLeading) {
                TextEditor(text: $model.commitMessage)
                    .font(.body)
                    .scrollContentBackground(.hidden)
                    .focused($editing)
                    .padding(.horizontal, 4)
                    .padding(.vertical, 6)
                if model.commitMessage.isEmpty {
                    Text(model.amendCommit ? "New message for the last commit" : "Commit message")
                        .foregroundStyle(.tertiary)
                        .padding(.horizontal, 9)
                        .padding(.vertical, 6)
                        .allowsHitTesting(false)
                }
            }
            .frame(height: 96)
            .background(Color(nsColor: .textBackgroundColor), in: RoundedRectangle(cornerRadius: 6))
            .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(.separator))

            HStack(spacing: 10) {
                Toggle("Amend", isOn: $model.amendCommit)
                    .toggleStyle(.checkbox)
                    .help("Replace the last commit with the staged changes and this message")
                summaryLength
                Spacer()
                Button(model.amendCommit ? "Amend" : "Commit") {
                    Task { await model.commit() }
                }
                .buttonStyle(.borderedProminent)
                .keyboardShortcut(.return, modifiers: .command)
                .disabled(!model.canCommit)
                .help(commitHelp)
            }
            .controlSize(.small)
        }
        .padding(10)
    }

    @ViewBuilder
    private var summaryLength: some View {
        let summary = model.commitMessage.split(separator: "\n", omittingEmptySubsequences: false).first ?? ""
        if !summary.isEmpty {
            Text("\(summary.count)")
                .font(.caption.monospacedDigit())
                .foregroundStyle(summary.count > Self.summaryLimit ? .orange : .secondary)
                .help("Characters in the first line. Keep it under \(Self.summaryLimit).")
        }
    }

    private var commitHelp: String {
        if model.commitMessage.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            return "Write a commit message first"
        }
        if model.status?.staged.isEmpty ?? true, !model.amendCommit {
            return "Stage some changes first"
        }
        return "Commit the staged changes (⌘↩)"
    }
}
