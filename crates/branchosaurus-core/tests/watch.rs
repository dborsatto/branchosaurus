// SPDX-License-Identifier: GPL-3.0-or-later

mod support;

use std::sync::mpsc;
use std::time::Duration;

use branchosaurus_core::WatchEvent;
use support::Fixture;

const TIMEOUT: Duration = Duration::from_secs(5);

/// Waits for the next report, skipping ones that don't match.
fn wait_for(receiver: &mpsc::Receiver<WatchEvent>, matches: impl Fn(&WatchEvent) -> bool) -> bool {
    let deadline = std::time::Instant::now() + TIMEOUT;
    while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
        match receiver.recv_timeout(left) {
            Ok(event) if matches(&event) => return true,
            Ok(_) => continue,
            Err(_) => return false,
        }
    }
    false
}

#[test]
fn reports_working_copy_index_and_ref_changes() {
    let fixture = Fixture::new();
    fixture.write(".gitignore", "*.log\n");
    fixture.write("a.txt", "a\n");
    fixture.commit_all("initial");
    let (sender, receiver) = mpsc::channel();
    let _watcher = fixture
        .repo()
        .watch(move |event| {
            let _ = sender.send(event);
        })
        .unwrap();
    // Let the watcher settle before changing anything.
    std::thread::sleep(Duration::from_millis(300));

    fixture.write("a.txt", "changed\n");
    assert!(wait_for(&receiver, |event| event.working_copy), "file edit");

    fixture.git(&["add", "a.txt"]);
    assert!(wait_for(&receiver, |event| event.index), "staging");

    fixture.git(&["branch", "new-branch"]);
    assert!(wait_for(&receiver, |event| event.refs), "new branch");

    // Ignored files don't count.
    while receiver.try_recv().is_ok() {}
    fixture.write("debug.log", "noise\n");
    assert!(
        !wait_for(&receiver, |event| event.working_copy),
        "ignored file"
    );
}
