// SPDX-License-Identifier: GPL-3.0-or-later

import AppKit
import SwiftUI

struct WelcomeView: View {
    @Binding var url: URL?
    @State private var recents: [URL] = RecentRepositories.load()

    var body: some View {
        VStack(spacing: 24) {
            VStack(spacing: 8) {
                Image(nsImage: NSApplication.shared.applicationIconImage)
                    .resizable()
                    .frame(width: 112, height: 112)
                    .accessibilityHidden(true)
                Text("Branchosaurus")
                    .font(.largeTitle.weight(.semibold))
                Text("A fast, native Git client")
                    .foregroundStyle(.secondary)
            }

            Button {
                if let chosen = chooseRepositoryFolder() {
                    open(chosen)
                }
            } label: {
                Label("Open Repository…", systemImage: "folder")
                    .frame(minWidth: 180)
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.large)

            if !recents.isEmpty {
                VStack(alignment: .leading, spacing: 6) {
                    Text("Recent")
                        .font(.headline)
                        .foregroundStyle(.secondary)
                    List(recents, id: \.self) { recent in
                        Button {
                            open(recent)
                        } label: {
                            HStack {
                                Image(systemName: "folder.fill")
                                    .foregroundStyle(.secondary)
                                VStack(alignment: .leading) {
                                    Text(recent.lastPathComponent)
                                    Text(recent.deletingLastPathComponent().path)
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                        .lineLimit(1)
                                        .truncationMode(.middle)
                                }
                            }
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    }
                    .listStyle(.bordered(alternatesRowBackgrounds: false))
                    .frame(height: min(CGFloat(recents.count) * 44 + 8, 240))
                }
                .frame(maxWidth: 420)
            }
        }
        .padding(40)
        .frame(minWidth: 520, minHeight: 440)
        .onAppear {
            if url == nil, let launch = LaunchArguments.take() {
                open(launch)
            }
        }
    }

    private func open(_ chosen: URL) {
        RecentRepositories.add(chosen)
        url = chosen
    }
}
