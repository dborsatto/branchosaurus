// SPDX-License-Identifier: GPL-3.0-or-later

//! What to show for a commit's author: their picture when there is one, otherwise their initials
//! on a color that stays the same for every commit they make.
//!
//! The core only says where a picture may be; apps download and cache it with their platform's
//! HTTP stack, and can skip downloading altogether.

use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Avatar {
    /// One or two letters from the name (or the email when there is no name), in upper case.
    pub initials: String,
    /// Index into the app's palette (wrapping around), derived from the email.
    pub color: u16,
    /// Where a picture may be: GitHub for its no-reply addresses, Gravatar otherwise. The Gravatar
    /// URL answers 404 for people without a picture. `None` without an email.
    pub image_url: Option<String>,
}

/// The avatar for an author. `pixels` is the picture size to ask for.
pub fn avatar(name: &str, email: &str, pixels: u32) -> Avatar {
    let email = email.trim().to_lowercase();
    Avatar {
        initials: initials(name, &email),
        color: color(if email.is_empty() { name } else { &email }),
        image_url: image_url(&email, pixels),
    }
}

fn initials(name: &str, email: &str) -> String {
    let source = if name.trim().is_empty() {
        email.split('@').next().unwrap_or_default()
    } else {
        name
    };
    // "Ada Lovelace" → AL, "J. R. R. Tolkien" → JT, "jean-luc.picard" → JP, "dependabot[bot]" → D.
    let words: Vec<char> = source
        .split(|c: char| c.is_whitespace() || matches!(c, '.' | '_' | '-'))
        .filter_map(|word| word.chars().find(|c| c.is_alphanumeric()))
        .collect();
    let letters = match words.as_slice() {
        [] => return "?".to_owned(),
        [only] => vec![*only],
        [first, .., last] => vec![*first, *last],
    };
    letters.into_iter().flat_map(char::to_uppercase).collect()
}

/// FNV-1a, so the color is the same on every platform and every run.
fn color(key: &str) -> u16 {
    let hash = key.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    });
    (hash >> 48) as u16
}

fn image_url(email: &str, pixels: u32) -> Option<String> {
    if email.is_empty() {
        return None;
    }
    // GitHub's no-reply addresses: `12345+octocat@…`, or `octocat@…` for older accounts.
    if let Some(user) = email.strip_suffix("@users.noreply.github.com") {
        return Some(match user.split_once('+') {
            Some((id, _)) if id.bytes().all(|byte| byte.is_ascii_digit()) => {
                format!("https://avatars.githubusercontent.com/u/{id}?s={pixels}&v=4")
            }
            _ => format!("https://github.com/{user}.png?size={pixels}"),
        });
    }
    let hash: String = Sha256::digest(email.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Some(format!(
        "https://gravatar.com/avatar/{hash}?s={pixels}&d=404"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_come_from_the_first_and_last_words() {
        let initials = |name: &str, email: &str| avatar(name, email, 64).initials;
        assert_eq!(initials("Ada Lovelace", ""), "AL");
        assert_eq!(initials("Junio C Hamano", ""), "JH");
        assert_eq!(initials("J. R. R. Tolkien", ""), "JT");
        assert_eq!(initials("Ævar Arnfjörð Bjarmason", ""), "ÆB");
        assert_eq!(initials("dependabot[bot]", ""), "D");
        assert_eq!(initials("  ", "jean-luc.picard@example.com"), "JP");
        assert_eq!(initials("", ""), "?");
    }

    #[test]
    fn colors_are_stable_and_follow_the_email() {
        let a = avatar("Ada", "ada@example.com", 64).color;
        assert_eq!(a, avatar("Ada Lovelace", " ADA@example.com ", 64).color);
        assert_ne!(a, avatar("Ada", "ada@example.org", 64).color);
    }

    #[test]
    fn gravatar_uses_the_sha256_of_the_normalised_email() {
        let url = avatar("", "MyEmailAddress@example.com ", 40).image_url;
        assert_eq!(
            url.as_deref(),
            Some(
                "https://gravatar.com/avatar/\
                 84059b07d4be67b806386c0aad8070a23f18836bbaae342275dc0a83414c32ee?s=40&d=404"
            )
        );
        assert_eq!(avatar("Nobody", "", 40).image_url, None);
    }

    #[test]
    fn github_noreply_addresses_use_github_avatars() {
        let url = |email: &str| avatar("", email, 64).image_url.unwrap();
        assert_eq!(
            url("49699333+dependabot[bot]@users.noreply.github.com"),
            "https://avatars.githubusercontent.com/u/49699333?s=64&v=4"
        );
        assert_eq!(
            url("octocat@users.noreply.github.com"),
            "https://github.com/octocat.png?size=64"
        );
    }
}
