// SPDX-License-Identifier: GPL-3.0-or-later

//! Runs the user's `git` executable. Every write and network operation goes through here so that
//! hooks, signing, credential helpers and SSH configuration behave exactly as in a terminal.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use crate::{CancelToken, Error, Result};

/// Environment variables that would redirect git away from the repository we point it at.
const INHERITED_ENV_TO_CLEAR: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_NAMESPACE",
    "GIT_PREFIX",
    // Prompts go through the app (see askpass.rs), not through whatever launched it, such as an
    // editor's terminal.
    "GIT_ASKPASS",
    "SSH_ASKPASS",
    "SSH_ASKPASS_REQUIRE",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct GitVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl GitVersion {
    /// Oldest git we support: `--porcelain=v2` with `-z` renames, `--pathspec-from-file`,
    /// and `--no-optional-locks` are all older, and 2.34 is what current LTS distributions ship.
    pub const MINIMUM: GitVersion = GitVersion {
        major: 2,
        minor: 34,
        patch: 0,
    };

    /// Parses `git version 2.54.0`, `git version 2.39.5 (Apple Git-154)` or
    /// `git version 2.47.1.windows.2`.
    pub fn parse(output: &str) -> Option<Self> {
        let version = output.trim().strip_prefix("git version ")?;
        let mut parts = version
            .split(|c: char| !c.is_ascii_digit())
            .take(3)
            .map(|part| part.parse::<u32>().ok());
        Some(Self {
            major: parts.next()??,
            minor: parts.next()??,
            patch: parts.next().flatten().unwrap_or(0),
        })
    }
}

impl fmt::Display for GitVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// A located and version-checked `git` executable.
#[derive(Clone, Debug)]
pub struct GitCli {
    program: PathBuf,
    version: GitVersion,
    env: Vec<(OsString, OsString)>,
}

impl GitCli {
    /// Finds a usable git. GUI apps don't inherit the login shell's `PATH`, so well-known install
    /// locations are tried before `PATH`. `BRANCHOSAURUS_GIT` overrides everything.
    pub fn locate() -> Result<Self> {
        if let Some(program) = std::env::var_os("BRANCHOSAURUS_GIT") {
            return Self::at(PathBuf::from(program));
        }
        let mut last_error = None;
        for candidate in candidates() {
            if !candidate.is_file() || is_unusable_stub(&candidate) {
                continue;
            }
            match Self::at(candidate) {
                Ok(git) => return Ok(git),
                Err(err) => last_error = Some(err),
            }
        }
        Err(last_error.unwrap_or_else(|| {
            Error::GitUnavailable(
                "git was not found. Install git, or set BRANCHOSAURUS_GIT to its path.".into(),
            )
        }))
    }

    /// Uses the git at `program`, failing if it is older than [`GitVersion::MINIMUM`].
    pub fn at(program: PathBuf) -> Result<Self> {
        let output = base_command(&program)
            .arg("version")
            .stdin(Stdio::null())
            .output()
            .map_err(|source| Error::Io {
                context: "running git",
                source,
            })?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let version = GitVersion::parse(&stdout).ok_or_else(|| {
            Error::GitUnavailable(format!(
                "{} did not report a version: {}",
                program.display(),
                stdout.trim()
            ))
        })?;
        if version < GitVersion::MINIMUM {
            return Err(Error::GitUnavailable(format!(
                "git {version} at {} is too old; Branchosaurus needs {} or newer",
                program.display(),
                GitVersion::MINIMUM
            )));
        }
        Ok(Self {
            program,
            version,
            env: Vec::new(),
        })
    }

    /// Sets an environment variable for every command, e.g. to isolate git from the user's
    /// configuration in tests.
    pub fn with_env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    pub fn program(&self) -> &Path {
        &self.program
    }

    pub fn version(&self) -> GitVersion {
        self.version
    }

    /// A `git` command running in `dir`, with an environment suitable for parsing its output.
    pub fn command(&self, dir: &Path) -> Command {
        let mut command = base_command(&self.program);
        command.current_dir(dir);
        command.envs(self.env.iter().map(|(key, value)| (key, value)));
        command
    }

    /// Runs git in `dir` and returns its stdout, or its stderr as the error.
    pub fn run<I, S>(&self, dir: &Path, args: I) -> Result<Vec<u8>>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.run_with_stdin(dir, args, None)
    }

    pub fn run_with_stdin<I, S>(&self, dir: &Path, args: I, stdin: Option<&[u8]>) -> Result<Vec<u8>>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut command = self.command(dir);
        command.args(args);
        let description = describe(&command);
        command
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().map_err(|source| Error::Io {
            context: "running git",
            source,
        })?;
        if let Some(input) = stdin {
            let mut pipe = child.stdin.take().expect("stdin is piped");
            pipe.write_all(input).map_err(|source| Error::Io {
                context: "writing to git",
                source,
            })?;
        }
        let output = child.wait_with_output().map_err(|source| Error::Io {
            context: "running git",
            source,
        })?;
        if output.status.success() {
            Ok(output.stdout)
        } else {
            // Some commands explain themselves on stdout (`git commit` with nothing to commit).
            let message = if output.stderr.trim_ascii().is_empty() {
                &output.stdout
            } else {
                &output.stderr
            };
            Err(Error::GitCommand {
                command: description,
                stderr: String::from_utf8_lossy(message).trim().to_owned(),
            })
        }
    }
}

/// Options for [`GitCli::run_streaming`].
pub(crate) struct Streaming<'a> {
    pub env: &'a [(OsString, OsString)],
    pub stdin: Option<&'a [u8]>,
    /// Called with every line git writes to stderr, including progress lines that git rewrites
    /// in place with `\r`.
    pub on_stderr_line: &'a mut dyn FnMut(&str),
    pub cancel: &'a CancelToken,
}

impl GitCli {
    /// Runs git, reporting stderr as it arrives and killing git if `cancel` is set. Returns
    /// stdout; on failure the error carries git's messages without its progress lines.
    pub(crate) fn run_streaming<I, S>(
        &self,
        dir: &Path,
        args: I,
        options: Streaming<'_>,
    ) -> Result<Vec<u8>>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let io_error = |context| move |source| Error::Io { context, source };
        let mut command = self.command(dir);
        command.args(args);
        command.envs(options.env.iter().map(|(key, value)| (key, value)));
        let description = describe(&command);
        command
            .stdin(if options.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().map_err(io_error("running git"))?;
        if let Some(input) = options.stdin {
            let mut pipe = child.stdin.take().expect("stdin is piped");
            pipe.write_all(input).map_err(io_error("writing to git"))?;
        }

        let mut stdout = child.stdout.take().expect("stdout is piped");
        let stdout_reader = std::thread::spawn(move || {
            let mut buffer = Vec::new();
            let _ = stdout.read_to_end(&mut buffer);
            buffer
        });
        let mut stderr = child.stderr.take().expect("stderr is piped");
        let (sender, receiver) = mpsc::channel::<Vec<u8>>();
        let stderr_reader = std::thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            while let Ok(read) = stderr.read(&mut chunk) {
                if read == 0 || sender.send(chunk[..read].to_vec()).is_err() {
                    break;
                }
            }
        });

        let mut pending: Vec<u8> = Vec::new();
        let mut messages = String::new();
        let mut handle_segment = |segment: &[u8], ends_line: bool| {
            let text = String::from_utf8_lossy(segment);
            if text.trim().is_empty() {
                return;
            }
            (options.on_stderr_line)(&text);
            if ends_line && !is_progress_line(&text) {
                messages.push_str(text.trim_end());
                messages.push('\n');
            }
        };
        loop {
            match receiver.recv_timeout(Duration::from_millis(50)) {
                Ok(chunk) => {
                    pending.extend_from_slice(&chunk);
                    while let Some(end) = pending.iter().position(|b| *b == b'\r' || *b == b'\n') {
                        let segment: Vec<u8> = pending.drain(..=end).collect();
                        handle_segment(&segment[..end], segment[end] == b'\n');
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            if options.cancel.is_cancelled() {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::Cancelled);
            }
        }
        handle_segment(&pending, true);
        let _ = stderr_reader.join();
        let status = child.wait().map_err(io_error("running git"))?;
        let stdout = stdout_reader.join().unwrap_or_default();
        if status.success() {
            Ok(stdout)
        } else {
            Err(Error::GitCommand {
                command: description,
                stderr: messages.trim().to_owned(),
            })
        }
    }
}

/// Lines like `Receiving objects:  45% (123/456)`, which git rewrites in place.
pub(crate) fn is_progress_line(line: &str) -> bool {
    line.contains("% (")
}

fn base_command(program: &Path) -> Command {
    let mut command = Command::new(program);
    for name in INHERITED_ENV_TO_CLEAR {
        command.env_remove(name);
    }
    command
        // Never block on a credential or passphrase prompt nobody can see.
        .env("GIT_TERMINAL_PROMPT", "0")
        // Stable, untranslated output for parsing.
        .env("LC_ALL", "C")
        .args(["-c", "core.quotepath=false", "-c", "color.ui=false"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

fn describe(command: &Command) -> String {
    command
        .get_args()
        .map(|arg| arg.to_string_lossy())
        .skip_while(|arg| arg == "-c" || arg.contains('='))
        .collect::<Vec<_>>()
        .join(" ")
}

fn candidates() -> Vec<PathBuf> {
    let exe = if cfg!(windows) { "git.exe" } else { "git" };
    let mut out: Vec<PathBuf> = Vec::new();
    if cfg!(target_os = "macos") {
        out.extend(
            [
                "/opt/homebrew/bin/git",
                "/usr/local/bin/git",
                "/usr/bin/git",
            ]
            .into_iter()
            .map(PathBuf::from),
        );
    } else if cfg!(windows) {
        for var in ["ProgramFiles", "ProgramW6432", "LOCALAPPDATA"] {
            if let Some(base) = std::env::var_os(var) {
                let base = PathBuf::from(base);
                out.push(base.join("Git").join("cmd").join(exe));
                out.push(base.join("Programs").join("Git").join("cmd").join(exe));
            }
        }
    } else {
        out.extend(
            ["/usr/bin/git", "/usr/local/bin/git"]
                .into_iter()
                .map(PathBuf::from),
        );
    }
    if let Some(path) = std::env::var_os("PATH") {
        out.extend(std::env::split_paths(&path).map(|dir| dir.join(exe)));
    }
    out
}

/// On macOS `/usr/bin/git` is a shim that opens an "install developer tools" dialog when the
/// Command Line Tools are missing. Detect that without running it.
fn is_unusable_stub(program: &Path) -> bool {
    if !cfg!(target_os = "macos") || program != Path::new("/usr/bin/git") {
        return false;
    }
    !Command::new("/usr/bin/xcode-select")
        .arg("-p")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(test)]
mod tests {
    use super::GitVersion;

    #[test]
    fn parses_versions() {
        let v = |major, minor, patch| GitVersion {
            major,
            minor,
            patch,
        };
        assert_eq!(GitVersion::parse("git version 2.54.0\n"), Some(v(2, 54, 0)));
        assert_eq!(
            GitVersion::parse("git version 2.39.5 (Apple Git-154)"),
            Some(v(2, 39, 5))
        );
        assert_eq!(
            GitVersion::parse("git version 2.47.1.windows.2"),
            Some(v(2, 47, 1))
        );
        assert_eq!(GitVersion::parse("git version 3.0"), Some(v(3, 0, 0)));
        assert_eq!(GitVersion::parse("hello"), None);
    }
}
