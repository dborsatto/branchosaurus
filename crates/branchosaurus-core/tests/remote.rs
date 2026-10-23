// SPDX-License-Identifier: GPL-3.0-or-later

//! Fetch, pull and push against a local bare repository, and credential prompts against a tiny
//! HTTP server that demands Basic authentication.

mod support;

use std::sync::{Arc, Mutex};

use branchosaurus_core::{
    CancelToken, CredentialPrompter, PullMode, RemoteOptions, RemoteProgress,
};
use support::Fixture;

/// Runs a remote operation without credentials or progress reporting.
fn run<T>(operation: impl FnOnce(RemoteOptions<'_>) -> branchosaurus_core::Result<T>) -> T {
    let cancel = CancelToken::new();
    let mut progress = |_: &RemoteProgress| {};
    operation(RemoteOptions {
        progress: &mut progress,
        credentials: None,
        cancel: &cancel,
    })
    .unwrap()
}

/// A repository with a commit, and a clone of it in `clone` tracking its bare copy.
struct Setup {
    upstream: Fixture,
    _bare: tempfile::TempDir,
    bare_path: String,
    clone: Fixture,
}

fn setup() -> Setup {
    let upstream = Fixture::new();
    upstream.write("README.md", "hello\n");
    upstream.commit_all("initial");
    let bare = upstream.bare_clone();
    let bare_path = bare.path().join("origin.git").display().to_string();
    let clone = Fixture::new();
    clone.git(&["remote", "add", "origin", &bare_path]);
    clone.git(&["fetch", "--quiet", "origin"]);
    clone.git(&[
        "switch",
        "--quiet",
        "--force",
        "-C",
        "main",
        "--track",
        "origin/main",
    ]);
    Setup {
        upstream,
        _bare: bare,
        bare_path,
        clone,
    }
}

#[test]
fn push_fetch_and_pull() {
    let Setup {
        upstream,
        bare_path,
        clone,
        _bare: _keep_bare,
    } = setup();
    let repo = clone.repo();

    // Push a new commit to the existing upstream.
    clone.write("README.md", "hello\nfrom the clone\n");
    clone.commit_all("from the clone");
    let target = run(|options| repo.push(false, options));
    assert!(!target.set_upstream);
    let remote_main = upstream.git(&["--git-dir", &bare_path, "rev-parse", "main"]);
    assert_eq!(remote_main.trim(), clone.rev_parse("HEAD"));

    // Someone else pushes; fetch sees it, pull brings it in.
    let other = Fixture::new();
    other.git(&["remote", "add", "origin", &bare_path]);
    other.git(&["fetch", "--quiet", "origin"]);
    other.git(&[
        "switch",
        "--quiet",
        "--force",
        "-C",
        "main",
        "--track",
        "origin/main",
    ]);
    other.write("OTHER.md", "other\n");
    let theirs = other.commit_all("from someone else");
    other.git(&["push", "--quiet", "origin", "main"]);

    run(|options| repo.fetch(None, options));
    assert_eq!(clone.rev_parse("origin/main"), theirs);
    assert_ne!(clone.rev_parse("HEAD"), theirs);
    run(|options| repo.pull(PullMode::FastForwardOnly, options));
    assert_eq!(clone.rev_parse("HEAD"), theirs);
    assert_eq!(clone.read("OTHER.md"), "other\n");
}

#[test]
fn pushing_a_new_branch_sets_its_upstream() {
    let Setup {
        upstream,
        bare_path,
        clone,
        _bare: _keep_bare,
    } = setup();
    let repo = clone.repo();
    repo.create_branch("feature", None, true).unwrap();
    clone.commit("feature work");

    let target = repo.push_target().unwrap();
    assert_eq!(target.remote, "origin");
    assert_eq!(target.remote_ref, "refs/heads/feature");
    assert!(target.set_upstream);

    run(|options| repo.push(false, options));
    assert_eq!(
        upstream
            .git(&["--git-dir", &bare_path, "rev-parse", "feature"])
            .trim(),
        clone.rev_parse("HEAD")
    );
    assert_eq!(
        clone.git(&["config", "branch.feature.remote"]).trim(),
        "origin"
    );
    assert!(!repo.push_target().unwrap().set_upstream);
}

#[test]
fn rejected_push_reports_git_s_message() {
    let Setup {
        bare_path,
        clone,
        _bare: _keep_bare,
        ..
    } = setup();
    let other = Fixture::new();
    other.git(&["remote", "add", "origin", &bare_path]);
    other.git(&["fetch", "--quiet", "origin"]);
    other.git(&[
        "switch",
        "--quiet",
        "--force",
        "-C",
        "main",
        "--track",
        "origin/main",
    ]);
    other.commit("theirs");
    other.git(&["push", "--quiet", "origin", "main"]);

    clone.commit("ours");
    let repo = clone.repo();
    let cancel = CancelToken::new();
    let error = repo
        .push(
            false,
            RemoteOptions {
                progress: &mut |_| {},
                credentials: None,
                cancel: &cancel,
            },
        )
        .unwrap_err();
    assert!(error.to_string().contains("rejected"), "{error}");
}

/// Records what it was asked and answers like a person would.
#[derive(Default)]
struct RecordingPrompter {
    asked: Mutex<Vec<(String, bool)>>,
}

impl CredentialPrompter for RecordingPrompter {
    fn ask(&self, prompt: &str, secret: bool) -> Option<String> {
        self.asked.lock().unwrap().push((prompt.to_owned(), secret));
        Some(if secret { "s3cret" } else { "ada" }.to_owned())
    }
}

#[cfg(unix)]
#[test]
fn credentials_are_asked_for_through_the_app() {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    // An HTTP "git server" that wants Basic auth, and records what it gets.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let authorizations = Arc::new(Mutex::new(Vec::<String>::new()));
    let server = std::thread::spawn({
        let authorizations = authorizations.clone();
        move || {
            for stream in listener.incoming().take(2) {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut authorization = None;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.strip_prefix("Authorization: ") {
                        authorization = Some(value.trim().to_owned());
                    }
                }
                let response = match authorization {
                    None => {
                        "HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"test\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    }
                    Some(value) => {
                        authorizations.lock().unwrap().push(value);
                        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    }
                };
                stream.write_all(response.as_bytes()).unwrap();
            }
        }
    });

    let fixture = Fixture::new();
    fixture.git(&[
        "remote",
        "add",
        "origin",
        &format!("http://127.0.0.1:{port}/repo.git"),
    ]);
    let repo = fixture.repo();
    let prompter = Arc::new(RecordingPrompter::default());
    let cancel = CancelToken::new();
    let result = repo.fetch(
        Some("origin"),
        RemoteOptions {
            progress: &mut |_| {},
            credentials: Some(prompter.clone()),
            cancel: &cancel,
        },
    );
    assert!(result.is_err(), "the server never serves a repository");
    server.join().unwrap();

    let asked = prompter.asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 2, "{asked:?}");
    assert!(
        asked[0].0.starts_with("Username for 'http://127.0.0.1"),
        "{asked:?}"
    );
    assert!(!asked[0].1);
    assert!(
        asked[1].0.starts_with("Password for 'http://ada@127.0.0.1"),
        "{asked:?}"
    );
    assert!(asked[1].1);
    // base64("ada:s3cret")
    assert_eq!(*authorizations.lock().unwrap(), ["Basic YWRhOnMzY3JldA=="]);
}

#[test]
fn without_a_prompter_credentials_fail_instead_of_hanging() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        use std::io::{Read, Write};
        let (mut stream, _) = listener.accept().unwrap();
        let mut buffer = [0u8; 4096];
        let _ = stream.read(&mut buffer);
        let _ = stream.write_all(b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"test\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    });
    let fixture = Fixture::new();
    fixture.git(&[
        "remote",
        "add",
        "origin",
        &format!("http://127.0.0.1:{port}/repo.git"),
    ]);
    let cancel = CancelToken::new();
    let error = fixture
        .repo()
        .fetch(
            Some("origin"),
            RemoteOptions {
                progress: &mut |_| {},
                credentials: None,
                cancel: &cancel,
            },
        )
        .unwrap_err();
    server.join().unwrap();
    assert!(
        error.to_string().to_lowercase().contains("username"),
        "{error}"
    );
}
