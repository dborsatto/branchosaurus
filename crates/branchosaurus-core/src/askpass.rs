// SPDX-License-Identifier: GPL-3.0-or-later

//! Lets git and ssh ask the person using the app for credentials.
//!
//! For each remote operation the core listens on a Unix socket in a private directory and points
//! `GIT_ASKPASS` and `SSH_ASKPASS` at the `branchosaurus-askpass` helper, which relays prompts
//! over that socket to a [`CredentialPrompter`] supplied by the shell. Credential helpers such as
//! the macOS keychain still run first, so people are only asked when git has nothing stored.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Asks the person for something git or ssh needs.
pub trait CredentialPrompter: Send + Sync {
    /// Shows `prompt` (e.g. `Password for 'https://ada@github.com': `) and waits for an answer.
    /// `secret` says whether the answer should be typed into a password field. Returns `None` if
    /// the person cancels.
    fn ask(&self, prompt: &str, secret: bool) -> Option<String>;
}

/// Whether a prompt asks for something that should not be shown on screen.
pub(crate) fn is_secret(prompt: &str) -> bool {
    let prompt = prompt.to_lowercase();
    ["password", "passphrase", "pin", "token", "secret"]
        .iter()
        .any(|word| prompt.contains(word))
}

/// The helper binary: next to the running executable (inside the app bundle) by default.
pub(crate) fn default_helper() -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "branchosaurus-askpass.exe"
    } else {
        "branchosaurus-askpass"
    };
    let helper = std::env::current_exe().ok()?.with_file_name(name);
    helper.is_file().then_some(helper)
}

#[cfg(unix)]
pub(crate) use unix::AskpassServer;

#[cfg(unix)]
mod unix {
    use std::fs::DirBuilder;
    use std::io::{ErrorKind, Read, Write};
    use std::os::unix::fs::DirBuilderExt;
    use std::os::unix::net::UnixListener;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::thread::JoinHandle;
    use std::time::Duration;

    use super::*;
    use crate::{Error, Result};

    /// Answers askpass requests while it is alive.
    pub(crate) struct AskpassServer {
        directory: PathBuf,
        socket: PathBuf,
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
    }

    impl AskpassServer {
        pub fn start(prompter: Arc<dyn CredentialPrompter>) -> Result<Self> {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let io = |source| Error::Io {
                context: "preparing the credential prompt",
                source,
            };
            // Short, because socket paths are limited to about 100 bytes.
            let directory = std::env::temp_dir().join(format!(
                "bsap-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            // Only this user may connect.
            DirBuilder::new()
                .mode(0o700)
                .create(&directory)
                .map_err(io)?;
            let socket = directory.join("s");
            let listener = UnixListener::bind(&socket).map_err(io)?;
            listener.set_nonblocking(true).map_err(io)?;

            let stop = Arc::new(AtomicBool::new(false));
            let thread = std::thread::spawn({
                let stop = stop.clone();
                move || {
                    while !stop.load(Ordering::Relaxed) {
                        match listener.accept() {
                            Ok((mut stream, _)) => {
                                // Accepted sockets inherit non-blocking mode on BSDs.
                                let _ = stream.set_nonblocking(false);
                                let mut prompt = String::new();
                                if stream.read_to_string(&mut prompt).is_err() {
                                    continue;
                                }
                                let prompt = prompt.trim_end();
                                let reply = match prompter.ask(prompt, is_secret(prompt)) {
                                    Some(answer) => format!("1{answer}"),
                                    None => "0".to_owned(),
                                };
                                let _ = stream.write_all(reply.as_bytes());
                            }
                            Err(err) if err.kind() == ErrorKind::WouldBlock => {
                                std::thread::sleep(Duration::from_millis(25));
                            }
                            Err(_) => break,
                        }
                    }
                }
            });
            Ok(Self {
                directory,
                socket,
                stop,
                thread: Some(thread),
            })
        }

        /// Environment for a git command that should prompt through this server.
        pub fn env(&self, helper: &Path) -> Vec<(OsString, OsString)> {
            vec![
                ("GIT_ASKPASS".into(), helper.into()),
                ("SSH_ASKPASS".into(), helper.into()),
                // OpenSSH 8.4+: use SSH_ASKPASS even though there is no terminal or X display.
                ("SSH_ASKPASS_REQUIRE".into(), "force".into()),
                (
                    "BRANCHOSAURUS_ASKPASS_SOCKET".into(),
                    self.socket.clone().into(),
                ),
            ]
        }
    }

    impl Drop for AskpassServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
            let _ = std::fs::remove_file(&self.socket);
            let _ = std::fs::remove_dir(&self.directory);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::is_secret;

    #[test]
    fn recognises_secret_prompts() {
        assert!(is_secret("Password for 'https://ada@github.com': "));
        assert!(is_secret(
            "Enter passphrase for key '/Users/ada/.ssh/id_ed25519': "
        ));
        assert!(!is_secret("Username for 'https://github.com': "));
        assert!(!is_secret(
            "Are you sure you want to continue connecting (yes/no/[fingerprint])? "
        ));
    }
}
