// SPDX-License-Identifier: GPL-3.0-or-later

import Foundation

extension BranchosaurusError {
    /// The message from the core, suitable for showing in the UI.
    public var message: String {
        switch self {
        case let .NotARepository(message), let .GitUnavailable(message),
            let .GitCommandFailed(message), let .NotFound(message), let .BareRepository(message),
            let .Cancelled(message), let .Unsupported(message), let .OutOfDate(message),
            let .Internal(message):
            return message
        }
    }

    public var isCancellation: Bool {
        if case .Cancelled = self { return true }
        return false
    }
}

extension HistoryRow: Identifiable {}

extension FileChange: Identifiable {
    public var id: Data { path.bytes }
}

extension StatusEntry: Identifiable {
    public var id: Data { path.bytes }
}

extension HistoryRow {
    public var authorDate: Date { Date(timeIntervalSince1970: TimeInterval(authorTime)) }
    public var commitDate: Date { Date(timeIntervalSince1970: TimeInterval(commitTime)) }
    public var shortID: String { String(id.prefix(8)) }
}

extension Signature {
    public var date: Date { Date(timeIntervalSince1970: TimeInterval(time)) }
}

extension CommitDetails {
    public var shortID: String { String(id.prefix(8)) }

    /// The message without its summary line.
    public var body: String {
        guard let newline = message.firstIndex(of: "\n") else { return "" }
        return message[newline...].trimmingCharacters(in: .whitespacesAndNewlines)
    }
}
