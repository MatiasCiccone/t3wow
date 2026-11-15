# Unlisting a release

A bad release can be unlisted: it stays on [the releases page](https://github.com/MatiasCiccone/t3wow/releases) with its files, but no companion is offered it, and the `/releases/latest` download link (in the README, the CurseForge description and changelogs) stops pointing at it. Then ship a patch release as usual, which is listed.

## How it works

- Every companion, from 0.4.0 on, asks GitHub for the repository's **latest** release and offers it only when it's newer than itself. It asks the API (`api.github.com/repos/…/releases/latest`), or the release page (`github.com/…/releases/latest`) when the API limits it. Both follow the same "latest" pointer, which never points at a prerelease. A release from the API that is marked prerelease is skipped as well.
- So **unlisting a release means marking it a prerelease**. A listed release is a published release that isn't a prerelease.
- GitHub doesn't reliably move "latest" on its own when the latest release becomes a prerelease. On t3wow-releases (below) the releases all share one commit, so GitHub can't order them by date at all. `scripts/unlist-release.mjs` therefore marks the newest listed release "latest" itself, then reads it back from the API address companions ask (the release page follows the same pointer).
- Nothing is deleted, rebuilt or re-signed. Each release keeps its `t3wow.exe`, `t3wow.exe.sha256` and `t3wow.exe.sha256.sig`. Whatever a companion installs is checked exactly as before: the SHA-256 and, once the release key is in, the signature.

What it means for each companion, with 0.9.1 unlisted and 0.9.0 the newest listed release:

| Companion | Before the patch | After 0.9.2 is out |
| --- | --- | --- |
| 0.9.0 or older | offered 0.9.0 if older, else nothing; never 0.9.1 | offered 0.9.2 |
| 0.9.1 (already updated) | stays on 0.9.1: companions never go back to an older version | offered 0.9.2 |

### Two repositories for a while

Companions up to 0.10.0 look for updates on [MatiasCiccone/t3wow-releases](https://github.com/MatiasCiccone/t3wow-releases/releases), where releases went while this repository was private. Later companions look here. Until t3wow-releases is archived, the Release workflow publishes every release to both, so **a release unlisted here must be unlisted on t3wow-releases too**: run each command below a second time with `--repo MatiasCiccone/t3wow-releases`.

## Unlist a release

You need the GitHub CLI (`gh`), signed in to an account that can edit the repository's releases.

1. See what it would do (this only reads):

   ```powershell
   node scripts/unlist-release.mjs v0.9.1
   ```

   It names the release it unlists and the release that becomes "latest".

2. Do it:

   ```powershell
   node scripts/unlist-release.mjs v0.9.1 --apply
   node scripts/unlist-release.mjs v0.9.1 --apply --repo MatiasCiccone/t3wow-releases
   ```

   Each ends with `Done. GitHub's latest release on <repository> is v0.9.0.`. If GitHub answers another release, it stops with an error. Once t3wow-releases is archived, the second command is no longer needed (and fails, since an archived repository can't be edited).

3. If the addon zip went to CurseForge, archive that file there too (the project's **Files** tab, then the file's menu). CurseForge's own updater doesn't read GitHub's releases.

4. Ship the fix: bump the version in the three files and push the tag, as in **Releasing** in the README. The Release workflow publishes the patch as "latest". Check it afterwards:

   ```powershell
   node scripts/unlist-release.mjs --check
   node scripts/unlist-release.mjs --check --repo MatiasCiccone/t3wow-releases
   ```

   Each prints `Latest: v0.9.2. Unlisted: v0.9.1.`, and fails if "latest" isn't the newest listed release.

5. The patch's notes list only the changes since the unlisted tag, but companions coming from 0.9.0 skip 0.9.1. Add what 0.9.1 brought to the patch's notes (`gh release edit v0.9.2 --repo MatiasCiccone/t3wow --notes-file <file>`, and the same with `--repo MatiasCiccone/t3wow-releases`, or edit them on the release pages).

## List it again

```powershell
node scripts/unlist-release.mjs v0.9.1 --relist --apply
```

It clears the prerelease mark and marks the newest listed release "latest". That's 0.9.1 itself only if nothing newer is listed. Add `--repo MatiasCiccone/t3wow-releases` for the other repository, as above.

## What it doesn't do

- **It doesn't revoke anything.** Unlisting is a mark on GitHub, not a signed statement. Anyone who can edit the releases (on this repository, a maintainer or a workflow with `contents: write`; on t3wow-releases, the `RELEASES_TOKEN` or the owner's GitHub account) could list the release again. What they could offer is still a release this project's CI built, and once the release key is in, signed. A companion still never installs a version older than or equal to its own. Revoking a release cryptographically would need a signed list that companions check, and nothing like that exists yet.
- **It doesn't take the release back from companions that already installed it.** Those companions stay on it until the patch is out.
- **The Release workflow still counts an unlisted release.** **Signed releases stay signed** looks at the most recent release whether it's listed or not, so a patch after a signed release that was unlisted must be signed too.
