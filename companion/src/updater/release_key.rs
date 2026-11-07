//! The public half of the release key, which the updater checks release signatures with.
//! It lives in its own file so setting it never touches updater.rs:
//! `node scripts/gen-signing-key.mjs <path>` writes the real key here (see
//! docs/release-signing.md). Keep the line below in this form: scripts/release-key.mjs reads
//! it to check the release workflow's T3WOW_SIGNING_KEY against it.
//!
//! All zeros is the placeholder: while it's here, updates need only a valid SHA-256. Once
//! the real key is here, every release must be signed with it.

pub(super) const RELEASE_PUBLIC_KEY: [u8; 32] = [0; 32];
