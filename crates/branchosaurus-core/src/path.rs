// SPDX-License-Identifier: GPL-3.0-or-later

use std::borrow::Cow;
use std::fmt;

use gix::bstr::{BStr, BString, ByteSlice};

/// A path relative to the repository root, as git stores it: `/`-separated bytes that are
/// usually, but not necessarily, valid UTF-8.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct RepoPath(BString);

impl RepoPath {
    pub fn new(bytes: impl Into<Vec<u8>>) -> Self {
        Self(BString::from(bytes.into()))
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_slice()
    }

    pub fn as_bstr(&self) -> &BStr {
        self.0.as_ref()
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.0.into()
    }

    /// The path for display. Invalid UTF-8 sequences are replaced with U+FFFD.
    pub fn display(&self) -> Cow<'_, str> {
        self.0.to_str_lossy()
    }

    /// The last path component.
    pub fn file_name(&self) -> &[u8] {
        let bytes = self.as_bytes();
        bytes.rsplit(|b| *b == b'/').next().unwrap_or(bytes)
    }
}

impl From<&BStr> for RepoPath {
    fn from(value: &BStr) -> Self {
        Self(value.to_owned())
    }
}

impl From<BString> for RepoPath {
    fn from(value: BString) -> Self {
        Self(value)
    }
}

impl From<&str> for RepoPath {
    fn from(value: &str) -> Self {
        Self(value.into())
    }
}

impl fmt::Debug for RepoPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

impl fmt::Display for RepoPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.display())
    }
}
