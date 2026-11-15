//! The public half of the release key, which the updater checks release signatures with.
//! It lives in its own file so setting it never touches updater.rs:
//! `node scripts/gen-signing-key.mjs <path>` writes the real key here (see
//! docs/release-signing.md). Keep the line below in this form: scripts/release-key.mjs reads
//! it to check the release workflow's T3WOW_SIGNING_KEY against it.
//!
//! All zeros is the placeholder: while it's here, updates need only a valid SHA-256. Once
//! the real key is here, every release must be signed with it.

pub(super) const RELEASE_PUBLIC_KEY: [u8; 32] = [
    0xba, 0x23, 0x03, 0xde, 0x60, 0x49, 0xb1, 0x35,
    0x78, 0xfe, 0x16, 0x35, 0x7f, 0x8a, 0xc7, 0x8e,
    0x11, 0x59, 0x5e, 0x83, 0xc8, 0x29, 0x73, 0x78,
    0xbf, 0x89, 0x8e, 0xff, 0xab, 0x6b, 0xe6, 0x54,
];
