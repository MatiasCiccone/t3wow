# Release signing

A T3 WoW release can be signed in two separate ways. They protect different things and are set up separately:

| | Release key | Authenticode (code signing) |
| --- | --- | --- |
| What gets signed | `t3wow.exe.sha256`, the release's checksum file | `t3wow.exe` itself |
| Who checks it | the companion's updater, before it installs an update | Windows (SmartScreen, the file's Properties → Digital Signatures) |
| What it stops | an update that someone other than this project's CI built, even if they can publish to `t3wow-releases` | the "Windows protected your PC / unrecognized app" warning, over time |
| Cost | free | about $10 a month, or more (see below) |
| Status | built; waiting for the key ([set it up](#set-up-the-release-key)) | deferred until the source is open ([decision](#decision-november-2026)); the workflow step stays off without its secrets |

## The release key

### How it works

The Release workflow signs `t3wow.exe.sha256` with an Ed25519 private key (the `T3WOW_SIGNING_KEY` secret) and publishes the signature as `t3wow.exe.sha256.sig`. Every companion built with the matching public key (in `companion/src/updater/release_key.rs`) installs an update only when that signature verifies, the checksum file names `t3wow.exe`, and the downloaded exe matches the checksum. An update that's unsigned, signed with another key or altered after signing is refused.

### What's already in place

- `scripts/gen-signing-key.mjs` makes the key pair: the private key to a file outside the repository (it refuses a path inside it), the public key into `companion/src/updater/release_key.rs`. It refuses to replace a key that's already there.
- `scripts/sign-release.mjs` signs the checksum in CI, after checking the secret is an Ed25519 key whose public half is the one the companion embeds. `--check` does only the check, before anything is built.
- `companion/src/updater.rs` verifies the signature (with `ring`) once `RELEASE_PUBLIC_KEY` isn't the all-zeros placeholder.
- `.github/workflows/release.yml`:
  - **Release key** fails the release when only one half is in place (the secret without the public key, or the reverse), since that release would be refused or would leave its companions unable to update. With neither, the release goes out unsigned with a warning.
  - **Signed releases stay signed** fails an unsigned release once an earlier release went out signed.
  - **Sign the checksum** writes `t3wow.exe.sha256.sig`, which is published with the release.
- `tests/release-signing.test.js` and the updater's tests check both sides against the same signed fixture (`companion/tests/fixtures/signed-release.json`).

### What's left

Only the key itself, which only you can make, and only on your own PC:

1. Make the key pair; the public half goes into `release_key.rs`.
2. Back up the private key off that PC.
3. Merge the public key through a pull request.
4. Add the private key as the `T3WOW_SIGNING_KEY` repository secret.
5. Release as usual. That release is the first signed one.

### Where the keys live, and why

- **Public key: committed**, in `companion/src/updater/release_key.rs`, and compiled in as a constant. It isn't secret. Committing it means every build has it, a local `npm run build:exe` included; it shows in review like any other change; and the workflow can check the secret against it before building. Injecting it from a CI variable at build time instead would make local builds and CI builds behave differently, and a mistyped variable would ship companions that refuse every later update. It lives in its own file, not in `updater.rs`, so setting it never conflicts with work on the updater.
- **Private key: a GitHub Actions repository secret**, `T3WOW_SIGNING_KEY`, which only the Release workflow reads. GitHub never shows a secret again and masks it in logs. Pull requests from forks don't get secrets. The other copies are your offline backups.
- Optionally, move the secret to a [deployment environment](https://docs.github.com/en/actions/managing-workflow-runs-and-deployments/managing-deployments/managing-environments-for-deployment) named `release`, limited to `v*` tags, and add `environment: release` to the release job: then only a tagged release can read it, not any workflow on any branch. Environments in private repositories need GitHub Pro or a paid plan.

### Set up the release key

Run the wizard from Git Bash, in a clean checkout of `main`:

```sh
bash scripts/release-signing-wizard.sh
```

It checks your tools, makes the key pair, walks you through the backup, checks the key locally, opens the pull request with the public key, sets the secret (with `gh`) and tells you what to look for in the first signed release. It asks before every commit, push and secret, and you can stop it with Ctrl-C and run it again: it remembers the key's path (in `~/.t3wow-release-signing.env`, never the key) and skips what's done.

The same steps by hand, in PowerShell:

1. Make the key pair, with the private key outside the repository:

   ```powershell
   New-Item -ItemType Directory -Force $HOME\keys | Out-Null
   node scripts/gen-signing-key.mjs $HOME\keys\t3wow-release-key.pem
   ```

   It writes the public key into `companion/src/updater/release_key.rs`.

2. Back up `t3wow-release-key.pem` in two places off this PC: a password manager entry (as an attachment) and an offline USB stick or encrypted archive, say. Keep it out of email, chat, plain cloud-synced folders and any repository. If it leaks, someone else can sign updates your companions install; if it's lost, see [Losing or replacing the key](#losing-or-replacing-the-key).

3. Check it:

   ```powershell
   $env:T3WOW_SIGNING_KEY = Get-Content -Raw $HOME\keys\t3wow-release-key.pem
   node scripts/sign-release.mjs --check
   Remove-Item Env:T3WOW_SIGNING_KEY
   npm test
   cargo test --manifest-path companion/Cargo.toml -- --ignored the_release_public_key_is_set
   ```

4. Commit `companion/src/updater/release_key.rs` on a branch and open a pull request. A `feat(companion): …` subject puts it in the release notes, which is right: it changes what companions accept.

5. Add the secret:

   ```powershell
   Get-Content -Raw $HOME\keys\t3wow-release-key.pem | gh secret set T3WOW_SIGNING_KEY --repo MatiasCiccone/t3wow
   ```

   Between this and the merge, a pushed tag stops at **Release key** (the secret without the public key). Nothing gets published that way; merge before tagging.

6. Merge, then release as usual. In the Release run, **Release key** says "The release key is set and matches the companion's public key", and the release on `t3wow-releases` lists `t3wow.exe.sha256.sig`.

7. Once you know a backup opens, you can delete the `.pem` from your PC: the workflow only needs the secret.

### The transition

Which companions check what:

| Companion | Checks |
| --- | --- |
| 0.4.0, 0.4.1 | the SHA-256 when the release has one; ignores the signature |
| 0.5.0 and later, built with the placeholder key | the SHA-256 (required); ignores the signature |
| the first release built with the real key, and every one after | the signature and the SHA-256, both required |

So nothing is stranded: every companion released so far updates to the first signed release, and enforces signatures from then on. There's no window where an enforcing companion meets an unsigned release, as long as the order above holds, and the workflow fails a release that would break it:

- the secret without the public key, or the public key without the secret: **Release key** fails;
- neither, after a release that went out signed: **Signed releases stay signed** fails.

What the transition can't do is protect companions that haven't updated yet: until a companion runs the first signed release, it trusts a matching SHA-256 from `t3wow-releases`, as it does today.

### Losing or replacing the key

There's one key, and companions trust only it. Neither the scripts nor the companion support switching keys yet, so:

- **Lost** (no backup, secret deleted): releases can't be signed with it again. Make a new key (`release_key.rs` back to `[0; 32]` first, then the steps above) and release. Companions built with the old key refuse that release, so their users have to download the new `t3wow.exe` by hand once; say so in the release notes.
- **Leaked**: the same, as soon as possible. Until those companions are replaced, whoever has the key can sign an update they'd install, provided they can also publish to `t3wow-releases` (which takes `RELEASES_TOKEN` or your GitHub account).
- **Planned rotation** would need the companion to accept a list of keys, and the workflow to sign with the old key a release that embeds the new one. That's a small change to make before it's needed, not now.

## Authenticode

### What it does, and what it doesn't

An Authenticode signature on `t3wow.exe`, made with a code-signing certificate from a CA that Windows trusts, means:

- Windows names you as the publisher: in the file's **Properties → Digital Signatures**, and in the SmartScreen dialog, which says **Publisher: <your name>** instead of "Unknown publisher".
- [Smart App Control](https://support.microsoft.com/en-us/topic/what-is-smart-app-control-285ea03d-fa88-4d56-882e-6698afdb7003) on Windows 11 lets the exe run. It blocks unsigned apps, and there's no per-app override.
- SmartScreen reputation can build on the certificate, and so carry over from one release to the next. Unsigned, every new exe starts from zero.

It **doesn't** make the "Windows protected your PC" warning go away on day one, whatever the certificate. Microsoft: "There is no exact threshold, but it can take several weeks and hundreds of clean installs from a wide audience" ([SmartScreen reputation](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation)). EV certificates stopped skipping that in 2024, and Microsoft now says paying for EV just to avoid SmartScreen "is no longer justified". For an app with T3 WoW's audience, expect the warning for a while after signing, with your name on it.

Two more things to know:

- The warning is about the **first download from a browser**. The browser marks the file as downloaded from the internet (Mark of the Web), and SmartScreen checks marked files. The companion's own updates are written by the companion, without that mark, so they shouldn't meet SmartScreen, signed or not. That's an inference, not something tested here. To check after an update: `Get-Item t3wow.exe -Stream Zone.Identifier` should find nothing.
- Every signature carries the certificate's subject: for an individual, your legal name and your city, region and country.

### The options (November 2026)

| Option | Who can get it | About a year | In CI | Verdict |
| --- | --- | --- | --- | --- |
| [Azure Artifact Signing](https://learn.microsoft.com/en-us/azure/artifact-signing/quickstart) (Trusted Signing until its GA in January 2026), Basic | individuals **only in the US or Canada**; organizations in the US, Canada, EU, UK and a few more | $120 ($9.99 a month) | official action, no hardware | best if you're eligible |
| [SSL.com](https://www.ssl.com/products/software-integrity/code-signing/iv/) IV certificate + [eSigner](https://www.ssl.com/guide/esigner-pricing-for-code-signing/) cloud signing (Tier 1, 20 signings a month) | individuals (government ID) | $309 ($129 + $180) | official action; the TOTP seed is a secret | **recommended outside the US and Canada** |
| SSL.com IV with the key in your own Azure Key Vault Premium HSM, signed with AzureSignTool | individuals | about $265 the first year | OIDC, no TOTP seed | cheaper, but an afternoon of setup |
| [Certum](https://shop.certum.eu/code-signing.html) Standard "in the Cloud" (SimplySign) | individuals | €209 | needs unofficial tools to get past its app-based codes | brittle in CI |
| Certum Open Source | individuals with a public project | €49 | same as above | not eligible: the source must be public, and the certificate is revoked if used for software sold commercially |
| [SignPath Foundation](https://signpath.org/terms) | OSI-licensed open-source projects with public source | free | good | not eligible while the source repository is private; the certificate names SignPath Foundation, not you |
| OV/EV on a USB token (DigiCert, Sectigo, GlobalSign…) | OV: organizations, some individuals; EV: organizations | $220–$600 | the token must be plugged into the machine that signs | no CI without a self-hosted runner; EV buys nothing for SmartScreen now |

Rules that apply to all of them:

- Since June 2023, the key must live in hardware: a token, an HSM or a CA's cloud HSM (CA/B Forum [Code Signing Baseline Requirements](https://github.com/cabforum/code-signing/blob/main/docs/CSBR.md) §6.2.7.4.2). No more `.pfx` files from public CAs.
- Since March 2026, a certificate lasts at most 460 days ([ballot CSC-31](https://cabforum.org/2025/11/17/ballot-csc-31-maximum-validity-reduction)). Multi-year purchases are reissued along the way.
- Signatures are timestamped, so they stay valid after the certificate expires.

### Recommendation

For a solo developer with a private source repository:

1. **If you live in the US or Canada: Azure Artifact Signing Basic.** It's the cheapest option, with Microsoft's own action, and nothing to renew by hand. One caveat, as of November 2026: Microsoft has acknowledged a bug, not yet fixed, that shows extra SmartScreen warnings for some files signed after it rotated its intermediate certificates ([issue #128](https://github.com/Azure/artifact-signing-action/issues/128)). The suggested workaround is to [submit each release to Microsoft](https://www.microsoft.com/wdsi/filesubmission).
2. **Anywhere else (Argentina included): an SSL.com IV certificate with eSigner, about $309 a year.** Working for a US company doesn't make you eligible for Artifact Signing as an individual: validation uses your own ID and your Azure billing address. SSL.com's IV certificate is for individuals with a government ID, and eSigner signs in its cloud HSM from GitHub Actions with no hardware. SSL.com doesn't list country limits, but ask them before buying whether they validate individuals in your country. This is what `release.yml` is wired for.
3. Wait, or skip it, if $120 to $309 a year isn't worth "Publisher: <your name>" in a warning that stays anyway for a while. The release key above protects updates either way; Authenticode is about first installs.

### Decision (November 2026)

**Skip Authenticode for now.** The source will be opened (under an OSI license) after alpha testing and the overhaul, which makes the free and nearly free options available. Until then:

- releases go out without Authenticode, and the README's "More info → Run anyway" note stays;
- the eSigner step in `release.yml` stays off (it only warns), since its secrets aren't set;
- the release key is set up regardless: it's free, and it's what protects updates.

### After open-sourcing

Revisit with these two, both of which need the source to be public:

- **[SignPath Foundation](https://signpath.org/terms), free.** It signs with its own certificate, so Windows shows "SignPath Foundation" as the publisher, not you. Conditions:
  - every component under an OSI-approved license, with no proprietary parts;
  - builds that SignPath can trace back to the public source, which means GitHub-hosted runners and their GitHub Action submitting the build for signing;
  - MFA for everyone on the project, with authors, reviewers and approvers named;
  - a "Code signing policy" section on the project's home page, and a privacy statement;
  - a manual approval for each release;
  - some reputation before they take an executable on.
- **Certum Open Source Code Signing, €49 a year.** Issued to you as a natural person, with "Open Source Developer" in the certificate. Its key is in Certum's cloud (SimplySign), whose codes come from a phone app, so CI needs unofficial tools. It gets revoked if used on software sold commercially.

SignPath is the better fit if its conditions are acceptable: free, and made for CI. Either one replaces the eSigner step (and its test) in `release.yml`. The paid options above stay open if neither works out.

### Set up Authenticode

With SSL.com eSigner, which `release.yml` already runs once its secrets exist. SSL.com's pages change; where these steps and theirs differ, theirs win ([eSigner automation guide](https://www.ssl.com/how-to/automate-esigner-ev-code-signing/)).

1. Buy an **IV (individual validation) code signing** certificate at [ssl.com](https://www.ssl.com/products/software-integrity/code-signing/iv/) and choose **eSigner** cloud signing for the key, not a USB token.
2. Complete the identity validation they ask for (a government ID and a call or video check).
3. Subscribe to eSigner (Tier 1 covers 20 signings a month; a release signs once). The first 30 days are free.
4. Enroll the certificate in eSigner. When it shows the QR code for your authenticator app, also copy the **secret code** under it: that's the TOTP seed the workflow uses to make one-time codes. Store it with your other secrets; anyone with it, your SSL.com username and password can sign as you.
5. Find the certificate's **credential ID**: CodeSignTool's `get_credential_ids` command lists your account's IDs, and the SSL.com dashboard shows it on the certificate's page.
6. Add the four secrets to this repository:

   ```powershell
   gh secret set ESIGNER_USERNAME --repo MatiasCiccone/t3wow
   gh secret set ESIGNER_PASSWORD --repo MatiasCiccone/t3wow
   gh secret set ESIGNER_CREDENTIAL_ID --repo MatiasCiccone/t3wow
   gh secret set ESIGNER_TOTP_SECRET --repo MatiasCiccone/t3wow
   ```

   Each one prompts for its value, so nothing lands in your shell history.

7. Release as usual. If any part is wrong, the release fails at **Authenticode (SSL.com eSigner)** or **Check the Authenticode signature**, before anything is published. Fix the secret and push the tag again (delete it first: `git push origin :v<version>`).
8. Check the published `t3wow.exe`: **Properties → Digital Signatures** lists your name with a timestamp.

At each renewal (at most every 460 days), check the credential ID and the TOTP seed still work, and update the secrets if SSL.com issued new ones.

### How it's wired into release.yml

- The job's `AUTHENTICODE` variable is `true` when the `ESIGNER_TOTP_SECRET` secret exists. `if:` can't test secrets directly, so the job reads it once.
- Without it, **Authenticode not set up** adds a warning to the run and the release goes out as before: CI and releases don't need the certificate.
- With it, **Authenticode (SSL.com eSigner)** signs `release/t3wow.exe` in place (`SSLcom/esigner-codesign`, pinned to the v1.3.2 commit), and **Check the Authenticode signature** fails the release unless `signtool verify /pa` passes, Windows reports the signature as valid, and it has a timestamp.
- Signing changes the exe's bytes, so it runs **before** **Package** computes the SHA-256, and before **Sign the checksum** signs that with the release key. The updater checks the exe that's actually published. `tests/release-signing.test.js` keeps that order.
- A missing username, password or credential ID with the TOTP seed set fails the signing step, rather than shipping unsigned.

Not run here yet, since that needs a real certificate: SSL.com's own CI runs the action on `windows-latest`, but this workflow hasn't signed with it. It runs on Node 20, which GitHub is retiring for actions. If it breaks, run SSL.com's CodeSignTool directly: `CodeSignTool sign -username … -password … -credential_id … -totp_secret … -input_file_path release\t3wow.exe -override`.

For local builds, `npm run build:exe` signs with `signtool` when `T3WOW_SIGN_THUMBPRINT`, `T3WOW_SIGN_PFX` or `T3WOW_SIGN_DLIB` is set (see the README); eSigner isn't one of them, since CI is where releases are signed.

### If you're eligible for Artifact Signing instead

One-time setup in the Azure portal (the [quickstart](https://learn.microsoft.com/en-us/azure/artifact-signing/quickstart) has the details):

1. Use a paid (pay-as-you-go) subscription whose billing account is of type **Individual**, with the legal name and address on your ID. Free and trial subscriptions are refused.
2. Register the `Microsoft.CodeSigning` resource provider, and create an Artifact Signing account (Basic) in a supported region. Note its endpoint, such as `https://eus.codesigning.azure.net/`.
3. Give yourself the **Artifact Signing Identity Verifier** role and complete the individual identity validation (an ID scan and a face check, through Microsoft Authenticator).
4. Create a **Public Trust** certificate profile.
5. Create an app registration (service principal) with a client secret, and give it **Artifact Signing Certificate Profile Signer** on the account. Owner and Contributor can't sign.
6. Add repository variables `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_SIGNING_ENDPOINT`, `AZURE_SIGNING_ACCOUNT`, `AZURE_CERT_PROFILE` (`gh variable set …`) and the secret `AZURE_CLIENT_SECRET`.

A client secret is the simplest login here. OIDC (federated credentials, no secret) would need one credential per tag, or a GitHub environment (a paid plan for private repositories), or Entra's flexible federated credentials, still in preview.

Then, in `release.yml`, replace the eSigner step with this one, and gate it and **Check the Authenticode signature** on `vars.AZURE_SIGNING_ACCOUNT != ''` instead of `env.AUTHENTICODE == 'true'` (repository variables can be tested in `if:`). Update the test that checks the order to the new step name.

```yaml
      - name: Authenticode (Azure Artifact Signing)
        if: vars.AZURE_SIGNING_ACCOUNT != ''
        uses: azure/artifact-signing-action@c7ab2a863ab5f9a846ddb8265964877ef296ee82 # v2.0.0
        with:
          azure-tenant-id: ${{ vars.AZURE_TENANT_ID }}
          azure-client-id: ${{ vars.AZURE_CLIENT_ID }}
          azure-client-secret: ${{ secrets.AZURE_CLIENT_SECRET }}
          endpoint: ${{ vars.AZURE_SIGNING_ENDPOINT }}
          signing-account-name: ${{ vars.AZURE_SIGNING_ACCOUNT }}
          certificate-profile-name: ${{ vars.AZURE_CERT_PROFILE }}
          files: ${{ github.workspace }}\release\t3wow.exe
          file-digest: SHA256
          timestamp-rfc3161: http://timestamp.acs.microsoft.com
          timestamp-digest: SHA256
          description: T3 WoW companion
          cache-dependencies: false
```

Artifact Signing's certificates last 72 hours and renew daily. The timestamp keeps old signatures valid, and SmartScreen reputation follows the validated identity rather than each certificate.

## Checklist for the next release

- [ ] Release key: run `bash scripts/release-signing-wizard.sh`, merge its pull request, confirm `T3WOW_SIGNING_KEY` is set.
- [ ] Authenticode: skipped until the source is open (see [Decision](#decision-november-2026)). The "Authenticode not set up" warning in the run is expected.
- [ ] Bump the version in the three files and push the tag.
- [ ] In the run: **Release key** says the key matches.
- [ ] On `t3wow-releases`: `t3wow.exe.sha256.sig` is there.
- [ ] From a 0.5.0 companion, **Update to x.y.z** installs it. After the update, a forged release (for instance, the same release with its `.sig` removed, on a test repository through `T3WOW_UPDATE_REPO` in a debug build) is refused.
