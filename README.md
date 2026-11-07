# t3wow

T3 Code inside World of Warcraft: Forever. You bring your harnesses to T3 Code; this brings T3 Code into WoW.

An in-game window lists your T3 threads from every machine your T3 Code reaches (this PC and its linked environments), shows a thread's conversation, and lets you message it, start a new thread, stop a run, and settle or unsettle a thread. That is all it does.

## What you need

- Windows, with WoW: Forever in **windowed** or **borderless windowed** mode (exclusive fullscreen blocks screen capture).
- T3 Code desktop running on the same PC. Threads on other machines show up when they are linked to it under **Settings → Environments → Linked environments**.

## Set up

1. Get `t3wow.exe` (built with `npm run build:exe`, see Develop) and double-click it. Windows may warn about an unrecognized app the first time, since it isn't signed: **More info → Run anyway**.
2. It finds WoW: Forever (a running client, Battle.net's install record, or the usual folders; it asks if it can't), puts the addon and its update slots into `Interface\AddOns`, and opens T3's sign-in page.
3. Approve the sign-in. If it asks for a pairing code, make one in T3 under **Settings → Sharing & access**. Choose **Full access** if your threads run in full access; T3 refuses messages to a thread that runs above the access you grant. The sign-in lasts 30 days; after that the game window says so, and the companion's next start opens the sign-in page again.
4. **Restart WoW completely** (not `/reload`) the first time: the game only sees addon files that existed when it started. In the AddOns list keep **T3 WoW** and the grey **T3 WoW slot** entries enabled.
5. Leave the companion's window open while you play.

Every start checks the addon files: anything an addon manager or a cleanup removed is put back, and a newer addon shipped with the companion replaces an older one. If it had to create files, restart WoW once more.

From a source checkout the same commands run with Node.js 20 or newer: `node companion/bin/t3wow.js` (or `run`) does all of the above; `install` reinstalls from scratch (`--wow "<path to _forever_>"` if WoW lives somewhere unusual), `login` signs in again, `status` checks the addon install, the screen capture, and one full read of your T3 threads on each machine, and `uninstall` removes the addon. `t3wow.exe` takes the same commands.

## In the game

`/t3`, the **T3 WoW** minimap button, or a key bound under **Options → Keybindings → AddOns → T3 WoW** opens the window. The button registers through LibDBIcon when another addon has loaded it, so collectors like MinimapButtonButton gather it with your other buttons. Right-click it (or `/t3 settings`, or **Settings** in the window) for the settings.

- Pick a thread on the left; **Open**, **Working**, **Snoozed** and **Settled** match T3's sidebar. The machine name is under each title.
- Type in the box and press **Enter** (Shift+Enter for a new line). Sending to a busy thread steers it or queues, as in T3.
- **New Thread** asks for the machine and project, then starts a thread with your message as its task.
- **Stop** interrupts a running thread. **Settle** / **Unsettle** moves it between Open and Settled.
- When a thread you are not looking at finishes, needs you or fails, a notification slides in near the top of the screen; click it to open the thread, right-click to dismiss. A line in chat and a sound come with it.
- **Settings** turns the notification, chat line and sound on or off, sets how long notifications stay (or until clicked), moves them (**Move notifications**, then drag), picks which outcomes notify (stopped threads are off by default), and shows or hides the minimap button.
- `/t3 status` prints the link state; `/t3 probe` tests which file checks work on your client.

Questions and permission prompts from an agent still need answering in T3 itself.

## How it connects

WoW addons cannot open sockets or read files while running. The companion (`companion/`) bridges that from outside the game, without touching the game process:

- **Game → companion:** the addon draws each command as a strip of colored 4×4-pixel squares in the screen's top-left corner, only while a command is waiting. The companion screen-captures that corner of the WoW window four times a second and decodes it.
- **Companion → game:** an addon marked load-on-demand reads its file from disk when it is loaded, and each loads once per UI session. `install` creates 200 such slot addons. The companion writes the latest state into the next slot, then turns a pre-made blank font file into a real one (a copy of a Windows font, Lucida Console by default); the addon sees that font load and loads the slot. The game stops noticing a font file it has checked for too long, so while waiting the addon moves to the next signal file every 25 seconds; the companion shares the PC clock, works out the same file, and makes it and its neighbours real. No `/reload` per message. Until its first update tells it which font to watch, or if fonts don’t work on a client, the addon loads slots on a timer instead, which is slower and uses up the 200 updates sooner. `/t3 status` says which mode it is in, and `/t3 probe` tests the file checks.
- **Companion → T3:** T3's MCP endpoint on this PC (`/mcp`, signed in with OAuth). Threads on linked machines go through T3's own environment links.

Every UI session has 200 updates; the window offers **Reload UI** when they run out. The companion only spends one when something you can see changed.

The addon uses only the documented addon API. The companion reads pixels in one corner of the WoW window and writes files in the AddOns folder; it never sends input to the game or reads its memory.

## Troubleshooting

- **"Waiting for the companion…"**: is `t3wow run` running? Is the game windowed? Is something covering the top-left corner?
- **"Slot addons blocked"**: enable the T3 WoW slot addons in the AddOns list, or allow out-of-date addons if the game's interface version changed (`install --interface <number>` rewrites it).
- `/t3 status` prints the transport state in chat.

## Develop

```sh
npm install
npm test
npm run build:exe   # dist/t3wow.exe, Windows only
```

The tests run the addon's encoder in a Lua VM (fengari) against the companion's decoder, and the companion's bridge against a fake T3.

`build:exe` bundles the companion with esbuild, embeds it with the addon files and the capture script into a copy of this machine's `node.exe` (Node's single executable applications, injected with postject), and writes `dist/t3wow.exe`, about 90 MB (`T3WOW_DIST` picks another folder, e.g. while `dist\t3wow.exe` is running).

It signs the exe when a code-signing certificate is configured, using `signtool.exe` from the Windows SDK and a DigiCert timestamp. Set one of:

- `T3WOW_SIGN_THUMBPRINT`: a certificate in your Windows certificate store, including one on a hardware token.
- `T3WOW_SIGN_PFX` (and `T3WOW_SIGN_PASSWORD`): a `.pfx` file.
- `T3WOW_SIGN_DLIB` and `T3WOW_SIGN_METADATA`: Azure Trusted Signing's dlib and its `metadata.json`.

Without one it builds unsigned, and Windows SmartScreen warns on first run.
