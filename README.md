<p align="center"><img src="assets/logo.png" alt="T3 WoW" width="160"></p>

# T3 WoW

T3 Code inside World of Warcraft: Forever. You bring your harnesses to T3 Code; this brings T3 Code into WoW.

An in-game window lists your T3 threads from every machine your T3 Code reaches (this PC and its linked environments), shows a thread's conversation, and lets you message it, answer an agent's questions, start a new thread, stop a run, and settle or unsettle a thread. When a thread finishes, fails or needs you, a notification slides in.

<p align="center"><img src="assets/screenshots/question.png" alt="The T3 WoW window: a thread whose agent asks two questions, with option buttons and Send answer" width="720"></p>

More screenshots are in [`assets/screenshots`](assets/screenshots).

## What you need

- Windows, with WoW: Forever in **windowed** or **borderless windowed** mode (exclusive fullscreen blocks screen capture).
- T3 Code desktop running on the same PC, or on another machine this PC can reach over the network: run `t3wow.exe login --origin http://<host>:<port>` once to point the companion at it (over a LAN, prefer Tailscale or HTTPS, since plain HTTP sends the sign-in token unencrypted). Threads on other machines also show up when they are linked to T3 under **Settings → Environments → Linked environments**.

## Set up

1. Get `t3wow.exe` (built with `npm run build:exe`, see Develop) and double-click it. It's a small native program (about 4 MB, nothing else to install) that lives in the system tray, next to the clock. Windows may warn about an unrecognized app the first time, since it isn't signed: **More info → Run anyway**.
2. It finds WoW: Forever (a running client, Battle.net's install record, or the usual folders; if it can't, its tray menu has **Choose WoW folder…**), puts the addon and its update slots into `Interface\AddOns`, and opens T3's sign-in page. The page opens by itself once per start; after that, **Sign in to T3…** in the tray menu opens it again, and choosing it while a sign-in is waiting starts that sign-in over.
3. Approve the sign-in. If it asks for a pairing code, make one in T3 under **Settings → Sharing & access**. Choose **Full access** if your threads run in full access; T3 refuses messages to a thread that runs above the access you grant. Read [Trust](#trust) before you do: the access you grant here is what commands from the game get. The sign-in lasts 30 days; after that the game window and the tray menu say so: choose **Sign in to T3…** in the tray menu.
4. **Restart WoW completely** (not `/reload`) the first time: the game only sees addon files that existed when it started. In the AddOns list keep **T3 WoW** and the grey **T3 WoW slot** entries enabled.
5. Leave T3 WoW running in the tray while you play. Its menu shows the T3 and WoW status (the WoW line also counts the updates used this UI session), and has **Sign out of T3** (or **Sign in to T3…**), **Choose WoW folder…**, **Reinstall addon**, **Match WoW's interface version…**, **Commands from the game** (see [Trust](#trust)), **Open log**, **Start with Windows** and the update check.

Every start checks the addon files: any of them an addon manager or a cleanup removed is put back, and a newer addon shipped with the companion replaces an older one. A newer addon (from CurseForge, say) is left alone and never patched with the companion's older files. If it had to create files, restart WoW once more.

Keep the addon and the companion at the same version. A newer addon with an older companion (a CurseForge update before the companion's), or the reverse until WoW restarts, still reads threads, sends messages and opens a thread it started. What needs both updated: a refused message or answer coming back to the box with its reason, recovery after WoW crashes, and switching to timed updates when a long game session runs out of signal files.

Sign-ins are kept per T3: switching between the T3 on this PC and one on another machine doesn't sign you out of either. They are saved in `%APPDATA%\t3wow\config.json`, encrypted for your Windows user, so copying that folder to another PC or Windows user means signing in again. A sign-in only ever goes to the T3 that issued it.

**Updates.** The companion looks for a newer GitHub release when it starts and every 6 hours. When there is one, a notification pops up from the tray and the menu offers **Update to x.y.z**, which downloads the new `t3wow.exe` and installs it only if the download matches the release's SHA-256 file. Release signing is being set up: once a companion has the project's release key built in, it also requires that SHA-256 file to carry a valid signature made with that key, and refuses a release that is unsigned, signed with another key or altered. Companions released before that check the SHA-256 only. It then swaps the exe in and restarts. The addon is part of the exe, so it updates along with it (restart WoW afterwards). An addon installed from CurseForge updates there; the companion never replaces a newer addon with an older one. Releases are published to the public [t3wow-releases](https://github.com/MatiasCiccone/t3wow-releases) repository while this one stays private.

`t3wow.exe` also works from a terminal:

- `run` runs in that console instead of the tray.
- `install` reinstalls the addon from scratch (`--wow "<path to _forever_>"` if WoW lives somewhere unusual, `--interface <number>` to set the interface version).
- `login` signs in again. `--origin <url>` points the companion at a T3 on another machine, and `--origin local` goes back to the T3 on this PC once it's running. The new T3 is saved only once its sign-in succeeds: a failed or cancelled sign-in leaves the companion on the T3 it had. A running companion picks up the new T3 and sign-in within 30 seconds.
- `status` checks the addon install and the T3 connection. `status --origin <url>` (or `local`) checks that T3 with its own saved sign-in, without switching to it. Other commands don't take `--origin`.
- `update` downloads and installs the newest release, with the same checks as the tray. It doesn't restart a companion running in the tray: quit it from its menu and start it again. Only one update runs at a time; a second one stops with "another update is already running".
- `uninstall` removes the addon.

## In the game

Each login and reload says in chat that T3 WoW loaded, then that it connected to T3 once the companion's first update arrives (and how many threads need you).

`/t3`, the **T3 WoW** minimap button, or a key bound under **Options → Keybindings → AddOns → T3 WoW** opens the window. The button registers through LibDBIcon when another addon has loaded it, so collectors like MinimapButtonButton gather it with your other buttons. Right-click it (or `/t3 settings`, or **Settings** in the window) for the settings.

- Pick a thread on the left; **Open**, **Working**, **Snoozed** and **Settled** match T3's sidebar, with up to 80 open, 25 snoozed and 25 settled threads per machine at first. A group with more ends in **Show more**, which lists more of it (twice as many each time, up to 320 open, 200 snoozed and 200 settled) until a new UI session (Reload UI or login). The machine name is under each title. Threads an agent started sit indented under the thread that started them, as in T3. A thread whose agent is asking you something shows **Needs you**.
- Type in the box and press **Enter** (Shift+Enter for a new line). Sending to a busy thread steers it or queues, as in T3. If T3 refuses a message, it comes back to the box (when the box is empty) with the reason. A message too long to send from the game stays in the box.
- **New Thread** asks for the machine and project, then starts a thread with your message as its task, and opens it. If the machine is unlinked or the project removed meanwhile, **Start** refuses and keeps your text.
- **Stop** interrupts a running thread. **Settle** / **Unsettle** moves it between Open and Settled.
- When a thread you are not looking at finishes, needs you or fails, a notification slides in near the top of the screen; click it to open the thread, right-click to dismiss. A line in chat and a sound come with it.
- **Settings** turns the notification, chat line and sound on or off, sets how long notifications stay (or until clicked) and how many stay on screen at once (the oldest makes room; a newer one for the same thread replaces it), moves them (**Move notifications**, then drag), picks which outcomes notify (stopped threads are off by default), and shows or hides the minimap button.

When an agent asks you a question, it shows at the end of the thread's conversation: pick an option (several, where it allows that) and press **Send answer**, or type your own answer in the box and press Enter. If the answer doesn't go through, **Send answer** can be pressed again. Permission prompts (approving a command or an edit) can't be answered through T3's tools by design: such a thread shows **Approve in T3**, and its thread view has an **Open in T3** button (the companion opens T3's own `t3code://` link). **Stop** cancels the run from the game. Clicking an approval's notification opens the thread in the game; turn on **Clicking an approval opens T3** in Settings to go straight to T3 instead.

When the tray's **Commands from the game** is set to **No new threads** or **Read-only**, the window greys out what that blocks (**Start**, or the box's Send, **Stop**, **Settle** and the answer buttons), says why, and keeps what you typed.

Commands:

- `/t3` opens and closes the window.
- `/t3 settings` opens the settings.
- `/t3 status` prints the link state: signals or timer, the slot it loads next and how many of the session's 200 updates are used, waiting commands, and the last few notifications.
- `/t3 minimap` shows or hides the minimap button.
- `/t3 probe` tests which file checks work on your client; include its output in a bug report.
- `/t3 trace` prints the link's latest trace lines, for debugging (the whole trace is saved with the addon's saved variables).

## Trust

Every addon you install runs in the same game as T3 WoW, and the strip of squares the addon draws (see How it connects) is a public format. Any addon can draw one, and the companion can't tell it from T3 WoW's. With the default setting, an addon could therefore do anything the window can: message your threads, answer their questions, stop or settle them, and start a new thread with any task on any machine your T3 reaches, with the access your sign-in grants. If you granted **Full access**, a started thread can run commands on your PCs.

The tray menu's **Commands from the game** sets how far the companion follows commands from the game:

- **Full** (the default): everything the window offers.
- **No new threads**: everything but starting threads.
- **Read-only**: nothing that changes T3. Messages, starts, answers, **Stop**, **Settle** and **Unsettle** are refused. Reading threads, opening one, loading earlier messages, **Show more**, **Refresh**, `/t3 probe` and **Open in T3** still work.

The setting applies at once and is saved in `config.json` (`gameCommands`). A refused command never reaches T3: the companion logs it, and the window gets it back as a failure with the reason and puts your text back (an addon older than the companion only shows the companion's note). Approvals can't be given from the game at any level.

The addon keeps its command queue and client key out of its saved variables while the game runs, so another addon can't just add commands to them; it can still draw a strip of its own, which only this setting limits. Install addons you trust, pick **No new threads** or **Read-only**, or grant T3 less access when you sign in.

**If you stream or record:** the strip encodes what you send from the game as plain data, not encrypted. Anyone with a recording of the game's top-left corner can decode the messages, answers and thread starts you send while it shows. Crop or cover that corner in your stream or recording. The strip is there only while a command is waiting, but that includes every message you send.

## How it connects

WoW addons cannot open sockets or read files while running. The companion (`companion/`, Rust) bridges that from outside the game, without touching the game process:

- **Game → companion:** the addon draws each command as a strip of colored 4×4-pixel squares in the screen's top-left corner, only while a command is waiting. The companion screen-captures that corner of the WoW window four times a second and decodes it.
- **Companion → game:** an addon marked load-on-demand reads its file from disk when it is loaded, and each loads once per UI session. The companion creates 200 such slot addons. The companion writes the latest state into the next slot, then turns a pre-made blank font file into a real one (a copy of a Windows font, Lucida Console by default); the addon sees that font load and loads the slot. The game stops noticing a font file it has checked for too long, so while waiting the addon moves to the next signal file every 25 seconds; the companion shares the PC clock, works out the same file, and makes it and its neighbours real. No `/reload` per message. Until its first update tells it which font to watch, or if fonts don't work on a client, the addon loads slots on a timer instead, which is slower and uses up the 200 updates sooner. `/t3 status` says which mode it is in, and `/t3 probe` tests the file checks.
- **Companion → T3:** T3's MCP endpoint (`/mcp`, signed in with OAuth). Threads on linked machines go through T3's own environment links.

Every UI session has 200 updates, and the companion only spends one when something you can see changed. From 190 used, the window says "Reload UI soon" and shows a **Reload UI** button, and a line in chat says so once. Once the addon loads update 200, the session is out: no more notifications until you Reload UI, which the window and chat say too.

Each game process also has 4000 signal files; a long day in one WoW session can use them up. After that, updates keep coming on the timer until WoW restarts, and the tray's WoW line says so. The addon also falls back to the timer if the test font never loads in 12 tries, or after 20000 font checks in one UI session (Reload UI resets that).

While the companion isn't running, the addon spaces its slot loads out (about once a minute after the first 20 seconds) instead of using up the session.

The addon uses only the documented addon API. The companion reads pixels in one corner of the WoW window and writes files in the AddOns folder; it never sends input to the game or reads its memory.

## Troubleshooting

- **"Waiting for the companion…"** or **"Companion not answering"**: is T3 WoW running in the tray? Is the game windowed? Is something covering the top-left corner? The tray's **Open log** shows what the companion sees.
- **"Slot addons blocked"**: enable the T3 WoW slot addons in the AddOns list. If the game's interface version changed with a patch, choose **Match WoW's interface version…** in the tray menu (your sign-in and settings stay; restart WoW afterwards), or allow out-of-date addons. The manual route: `/dump select(4, GetBuildInfo())` in the game prints the number, and `t3wow.exe install --interface <number>` rewrites it.
- **"Couldn't write the game's update files: …"** on the tray's WoW line: writing the slot or signal files keeps failing, usually because antivirus or folder permissions block the AddOns folder.
- **A command from the game does nothing and the window says it's off**: check **Commands from the game** in the tray menu.
- **Asked to sign in again after a restart**: if `config.json` was damaged, the companion keeps it as `config.json.bad` next to it and starts with default settings. Copying `%APPDATA%\t3wow` from another PC or Windows user also needs a new sign-in.
- `/t3 status` prints the transport state in chat.

## Develop

Needs Node.js 20+ (for the addon's tests and the release scripts) and Rust 1.90 or newer with the MSVC toolchain (for the companion).

```sh
npm ci
npm test            # the addon's Lua (in fengari), release signing, and the companion (cargo test)
npm run build:exe   # dist/t3wow.exe, Windows only
```

- `addon/T3WoW`: the addon (Lua). `tests/` runs it in a Lua VM: the link and the window against a simulated game, and the contract with the companion. It also checks the release signing scripts against the signed release the companion's tests verify.
- `companion/`: the companion (Rust): screen capture (`capture.rs`), the T3 client and sign-in (`t3.rs`), settings and saved sign-ins (`config.rs`), the bridge (`bridge.rs`), the addon files (`slots.rs`, with the addon embedded), updates (`updater.rs`) and the tray (`tray.rs`). `companion/tests/fixtures` holds the frames, inbox files and a signed release both sides are tested against.
- `scripts/`: the exe build and the release key scripts.
- `assets/`: the logo (`logo.png`, and `curseforge-avatar.png` at 400×400).

Debug builds read `T3WOW_UPDATE_REPO` (`owner/repo`) to test updates against another repository; release builds ignore it.

`build:exe` runs `cargo build --release` and copies the result to `dist/t3wow.exe` (`T3WOW_DIST` picks another folder). It signs the exe when a code-signing certificate is configured, using `signtool.exe` from the Windows SDK and a DigiCert timestamp. Set one of:

- `T3WOW_SIGN_THUMBPRINT`: a certificate in your Windows certificate store, including one on a hardware token.
- `T3WOW_SIGN_PFX` (and `T3WOW_SIGN_PASSWORD`): a `.pfx` file.
- `T3WOW_SIGN_DLIB` and `T3WOW_SIGN_METADATA`: Azure Trusted Signing's dlib and its `metadata.json`.

Without one it builds unsigned, and Windows SmartScreen warns on first run. This code signing is separate from the release key below, which the updater checks.

**CI** (`.github/workflows/ci.yml`) runs the tests, clippy and a release build on Windows for every pull request and every push to `main`, and attaches the built `t3wow.exe` to the run. Every action in the workflows is pinned to a commit SHA, and Dependabot keeps the pins current.

**Repository rules:** pull requests merge by rebase only (linear history), branches are deleted after merging, and Dependabot opens security fixes and weekly updates. `.github/rulesets/main.json` protects `main`: no force pushes or deletion, linear history, changes only through pull requests that pass CI (`test`) and get the code owner's review (`.github/CODEOWNERS`); repository admins can bypass. GitHub enforces rulesets on public repositories, or on private ones with GitHub Pro; apply it with `gh api repos/MatiasCiccone/t3wow/rulesets --input .github/rulesets/main.json`.

**Releasing:** set the same version in `companion/Cargo.toml`, `addon/T3WoW/T3WoW.toc` and `package.json`, then push a tag (`git tag v0.4.0 && git push origin v0.4.0`). The Release workflow checks that all three match the tag and checks the release key, runs the tests, builds on Windows, signs the SHA-256 file once signing is set up, and publishes `t3wow.exe`, `t3wow.exe.sha256`, `t3wow.exe.sha256.sig` when signed (what the updater downloads and checks) and `T3WoW-v0.4.0.zip` (the addon alone, for CurseForge) to the public `MatiasCiccone/t3wow-releases`. The notes list the `feat` and `fix` commit subjects since the previous tag, without their prefixes; other commits stay out. It needs two secrets:

- `RELEASES_TOKEN`: a fine-grained token with Contents read and write on `t3wow-releases` only.
- `T3WOW_SIGNING_KEY`: the release key's private half (until it's set up, see below, releases go out unsigned).

The same zip is uploaded to the [CurseForge project](https://www.curseforge.com/wow/addons) (ID 1788778) with the `CURSEFORGE_TOKEN` secret, filed under the WoW Forever version whose API version is the `.toc`'s `## Interface`; without that secret, this step is skipped.

**The release key**, set up once, on your own PC. Until both halves are in place, releases go out unsigned (the workflow says so in a warning) and companions check only the SHA-256. With only one half in place, the Release workflow stops at its **Release key** step, since that release would be refused or would leave its companions unable to update.

1. Make the key pair, with the private half outside the repository (the folder must exist; the script refuses a path inside the repository or an existing file):
   `node scripts/gen-signing-key.mjs C:\Users\<you>\keys\t3wow-release-key.pem`
   It prints a line `const RELEASE_PUBLIC_KEY: [u8; 32] = [ … ];`.
2. In `companion/src/updater.rs`, replace `const RELEASE_PUBLIC_KEY: [u8; 32] = [0; 32];` with that line and update the placeholder comment above it. Keep the line in that form: `scripts/sign-release.mjs` reads the key from it. Then `cargo test -- --ignored the_release_public_key_is_set` (in `companion/`) and `npm test` must pass.
3. Add the private half as the secret: `gh secret set T3WOW_SIGNING_KEY --repo MatiasCiccone/t3wow < C:\Users\<you>\keys\t3wow-release-key.pem`
4. Back the `.pem` up somewhere safe and offline. If it's lost, no new release can update existing companions; everyone would have to download a new `t3wow.exe` by hand.
5. Optionally, check it locally: `$env:T3WOW_SIGNING_KEY = Get-Content -Raw <path>; node scripts/sign-release.mjs --check`, then `Remove-Item Env:T3WOW_SIGNING_KEY`.

Companions built with the placeholder key (0.4.1, and releases until the key is in) check only the SHA-256 and ignore the signature, so they still update to signed releases. Every companion built with the real key requires the signature.

## License

MIT, see [LICENSE](LICENSE). The addon carries a copy in its folder.
