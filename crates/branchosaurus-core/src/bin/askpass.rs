// SPDX-License-Identifier: GPL-3.0-or-later

//! Started by git and ssh (as `GIT_ASKPASS` / `SSH_ASKPASS`) when they need a username, password,
//! passphrase or confirmation. Forwards the prompt to the Branchosaurus app over the socket named
//! in `BRANCHOSAURUS_ASKPASS_SOCKET` and prints the answer, which is how the askpass protocol
//! returns it.

use std::process::ExitCode;

fn main() -> ExitCode {
    let prompt = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    match ask(&prompt) {
        Some(answer) => {
            println!("{answer}");
            ExitCode::SUCCESS
        }
        None => ExitCode::FAILURE,
    }
}

#[cfg(unix)]
fn ask(prompt: &str) -> Option<String> {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;

    let socket = std::env::var_os("BRANCHOSAURUS_ASKPASS_SOCKET")?;
    let mut stream = UnixStream::connect(socket).ok()?;
    stream.write_all(prompt.as_bytes()).ok()?;
    stream.shutdown(std::net::Shutdown::Write).ok()?;
    let mut reply = Vec::new();
    stream.read_to_end(&mut reply).ok()?;
    // `1` followed by the answer, or `0` when the person cancelled.
    match reply.split_first() {
        Some((b'1', answer)) => String::from_utf8(answer.to_vec()).ok(),
        _ => None,
    }
}

#[cfg(not(unix))]
fn ask(_prompt: &str) -> Option<String> {
    None
}
