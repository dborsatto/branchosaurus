// SPDX-License-Identifier: GPL-3.0-or-later

#if DEBUG
    import AppKit

    /// Debug builds only: saves PNGs of the app's own windows, for checking the UI from scripts
    /// without screen-recording permissions.
    ///
    /// `BRANCHOSAURUS_SNAPSHOT=/tmp/shot` writes `/tmp/shot-0.png`, `/tmp/shot-1.png`, … after
    /// `BRANCHOSAURUS_SNAPSHOT_DELAY` seconds (default 4). `BRANCHOSAURUS_SNAPSHOT_QUIT=1` quits
    /// afterwards. `BRANCHOSAURUS_APPEARANCE=dark|light` overrides the system appearance.
    enum DebugSnapshot {
        @MainActor static func scheduleIfRequested() {
            let environment = ProcessInfo.processInfo.environment
            switch environment["BRANCHOSAURUS_APPEARANCE"] {
            case "dark": NSApplication.shared.appearance = NSAppearance(named: .darkAqua)
            case "light": NSApplication.shared.appearance = NSAppearance(named: .aqua)
            default: break
            }
            guard let prefix = environment["BRANCHOSAURUS_SNAPSHOT"] else { return }
            let delay = environment["BRANCHOSAURUS_SNAPSHOT_DELAY"].flatMap(Double.init) ?? 4
            DispatchQueue.main.asyncAfter(deadline: .now() + delay) {
                MainActor.assumeIsolated {
                    capture(prefix: prefix)
                    if environment["BRANCHOSAURUS_SNAPSHOT_QUIT"] == "1" {
                        NSApp.terminate(nil)
                    }
                }
            }
        }

        @MainActor private static func capture(prefix: String) {
            let windows = NSApp.windows.filter { $0.isVisible && $0.contentView != nil }
            for (index, window) in windows.enumerated() {
                let path = "\(prefix)-\(index).png"
                // `screencapture -l` captures just this window, as the window server draws it.
                let process = Process()
                process.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
                process.arguments = ["-x", "-o", "-l", String(window.windowNumber), path]
                process.standardError = FileHandle.nullDevice
                try? process.run()
                process.waitUntilExit()
                // Without screen-recording permission (e.g. when started by `open`), draw the
                // window's views instead. Materials and vibrancy look flat that way.
                if !FileManager.default.fileExists(atPath: path),
                    let view = window.contentView?.superview ?? window.contentView,
                    let bitmap = view.bitmapImageRepForCachingDisplay(in: view.bounds)
                {
                    view.cacheDisplay(in: view.bounds, to: bitmap)
                    try? bitmap.representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: path))
                }
                FileHandle.standardError.write(Data("snapshot: \(window.title) → \(path)\n".utf8))
            }
        }
    }
#endif
