# Contributing

Issues and pull requests are welcome. For a larger change, open an issue first, so the approach is agreed before the work. A security problem goes through [SECURITY.md](SECURITY.md), not an issue.

## Run the tests

You need Node.js 20 or later (CI uses 22) and stable Rust.

```sh
npm ci
npm test
cargo clippy --manifest-path companion/Cargo.toml --all-targets -- -D warnings
```

`npm test` runs the addon's Lua tests (in fengari), the release signing tests and the companion's (`cargo test`). CI runs the same on Windows (`test-windows`) and macOS (`test-macos`), and clippy must pass with no warnings.

Some checks need more than a checkout:

- **Live tests** (`npm run test:live`) run against a real T3 Code the companion is signed in to: see **Live tests** in the [README](README.md#develop).
- A change to the companion or the addon gets the [smoke test in the game](docs/smoke-test.md) before a release.
- Moving to a new T3 Code release: [docs/t3-update.md](docs/t3-update.md).

## Pull requests

- One concern per pull request.
- Pull requests merge by rebase only, so each commit lands on `main` as it is.
- Commit subjects follow the repository's style: `feat: …`, `fix: …`, `docs: …`, `ci: …`, `build: …`, `chore: …` (`git log --oneline` shows plenty). The `feat` and `fix` subjects, without their prefix, become the release notes, so write them for the people who use T3 WoW: what changed for them. The other types stay out of the notes.
- CI must pass, and the code owner reviews ([.github/CODEOWNERS](.github/CODEOWNERS)).

## Never commit secrets

No tokens, keys, certificates (`.pem`, `.p12`), passwords, or your own `config.json` or logs. The release key and the signing certificates live only in GitHub secrets and the maintainer's offline backups ([docs/release-signing.md](docs/release-signing.md)); `scripts/gen-signing-key.mjs` refuses to write a key inside the repository. If one got pushed by mistake, treat it as leaked and replace it: deleting it in a later commit doesn't take it out of the history. If it's one of the project's, report it privately as [SECURITY.md](SECURITY.md) says.
