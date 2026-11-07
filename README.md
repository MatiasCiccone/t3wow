# t3wow

T3 Code inside World of Warcraft: Forever. You bring your harnesses to T3 Code; this brings T3 Code into WoW.

An in-game window lists your T3 threads from every machine your T3 Code reaches (this PC and its linked environments), shows a thread's conversation, and lets you message it, start a new thread, stop a run, and settle or unsettle a thread. That is all it does.

## What you need

- Windows, with WoW: Forever in **windowed** or **borderless windowed** mode (exclusive fullscreen blocks screen capture).
- T3 Code desktop running on the same PC. Threads on other machines show up when they are linked to it under **Settings → Environments → Linked environments**.
- Node.js 20 or newer.
- Game sound enabled (the addon checks a silent sound file to know an update is ready; without sound it falls back to slower polling).

## Set up

```powershell
git clone git@github.com:MatiasCiccone/t3wow.git
cd t3wow
node companion/bin/t3wow.js install     # copies the addon and its update slots into WoW
node companion/bin/t3wow.js login       # approve in the browser; pick Full access
node companion/bin/t3wow.js run         # leave this running while you play
```

`install` finds `C:\Program Files (x86)\World of Warcraft\_forever_`; pass `--wow "<path to _forever_>"` if WoW lives elsewhere. Then **restart WoW completely** (not `/reload`): the game only sees addon files that existed when it started. In the AddOns list keep **T3 Code** and the grey **T3 WoW slot** entries enabled.

`login` opens T3's sign-in page. If it asks for a pairing code, make one in T3 under **Settings → Sharing & access**. Choose **Full access** if your threads run in full access; T3 refuses messages to a thread that runs above the access you grant. The sign-in lasts 30 days.

`status` checks everything: the addon install, the screen capture, and one full read of your T3 threads on each machine.

## In the game

`/t3` (or a key bound under **Options → Keybindings → AddOns → T3 Code**) opens the window.

- Pick a thread on the left; **Working**, **Open** and **Settled** match T3's sidebar. The machine name is under each title.
- Type in the box and press **Enter** (Shift+Enter for a new line). Sending to a busy thread steers it or queues, as in T3.
- **New Thread** asks for the machine and project, then starts a thread with your message as its task.
- **Stop** interrupts a running thread. **Settle** / **Unsettle** moves it between Open and Settled.
- When a thread you are not looking at finishes or needs you, a line appears in chat.

Questions and permission prompts from an agent still need answering in T3 itself.

## How it connects

WoW addons cannot open sockets or read files while running. The companion (`companion/`) bridges that from outside the game, without touching the game process:

- **Game → companion:** the addon draws each command as a strip of colored 4×4-pixel squares in the screen's top-left corner, only while a command is waiting. The companion screen-captures that corner of the WoW window four times a second and decodes it.
- **Companion → game:** an addon marked load-on-demand reads its file from disk when it is loaded, and each loads once per UI session. `install` creates 200 such slot addons. The companion writes the latest state into the next slot, then flips a pre-made empty `.wav` to a valid silent one; the addon sees it can now play that file and loads the slot. No `/reload` per message.
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
```

The tests run the addon's encoder in a Lua VM (fengari) against the companion's decoder, and the companion's bridge against a fake T3.
