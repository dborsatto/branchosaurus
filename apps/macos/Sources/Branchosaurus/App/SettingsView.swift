// SPDX-License-Identifier: GPL-3.0-or-later

import SwiftUI

/// The Settings window (⌘,).
struct SettingsView: View {
    @AppStorage(AvatarStore.loadImagesKey) private var loadAvatars = true

    var body: some View {
        Form {
            Section {
                Toggle("Show authors’ pictures", isOn: $loadAvatars)
                Text(
                    "Pictures come from Gravatar, which is sent a hash of each author’s email "
                        + "address, or from GitHub for its no-reply addresses. Without them, the "
                        + "commit graph shows authors’ initials."
                )
                .font(.callout)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            } header: {
                Text("Commit Graph")
            }
        }
        .formStyle(.grouped)
        .frame(width: 460)
    }
}
