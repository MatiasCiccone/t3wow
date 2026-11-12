# Smoke test in the game

`npm test` and `npm run test:live` cover the companion against T3 and the addon in a simulated game. What they can't reach is WoW itself and Windows: the screen strip, signal fonts and slot addons, pictures drawn with Edge, the clipboard and Ctrl+V, sounds, the tray and updates. This is the check for those, by hand, in about ten minutes.

**When:** before a release whose changes touch the companion or the addon (not for docs, tests or CI alone), and after a WoW patch. A T3 Code update alone needs only `npm run test:live`.

## Before you start

1. `npm test` and `npm run test:live` pass.
2. `npm run build:exe`, then quit the companion in the tray and start `dist\t3wow.exe`.
3. The WoW install is shared: another worktree's build may have replaced the addon, or still be running in the tray. The tray's **Open log** starts each run with `T3 WoW companion <version>`, and in the game `/dump C_AddOns.GetAddOnMetadata("T3WoW", "Version")` prints the addon's. Both must be the build under test.
4. Threads you start go in the **T3 WoW live tests** project; archive them in T3 when you're done.

## Core: every time

| Step | Expect | What it checks |
| --- | --- | --- |
| Start the companion | Tray notification; the menu's T3 and WoW lines say connected | sign-in, finding WoW, addon files |
| Start WoW from scratch (not `/reload`) | Chat says T3 WoW loaded, then connected to T3 | slot addons, first update |
| `/t3 status` | **signals**, not timer | signal fonts |
| `/t3`, open a thread | The conversation shows within a second or two | strip read, updates arriving |
| Send a message to it | It shows in T3, and the reply in the game | the strip carrying commands |
| **New Thread** in the test project | It starts and opens in place of the composer | starting threads from the game |
| Copy a screenshot (Win+Shift+S), Ctrl+V in the box, send | Thumbnail above the box; the image in T3's message | clipboard, Ctrl+V hook, upload |
| Ask an agent to show a small chart with `html_render` | It shows in the conversation as a picture | Edge drawing, image slots |
| Let a thread you're not viewing finish | Notification slides in, chat line, sound | alerts |
| `/reload` | Connects again; no notification sound for old news | new UI session |

## More, for what the change touched

| Changed | Also check |
| --- | --- |
| `capture.rs`, `codec.rs`, `Codec.lua` | Long messages, an answer to a question, Stop: commands of every size get through |
| `slots.rs`, `Link.lua`, signals | `/t3 probe`; a long session (`/t3 status` counts the 200 updates); WoW running when the addon files change: the window's top line says to restart WoW; the AddOns list in the game shows T3 WoW as one entry, with no slot rows under it |
| `render.rs`, pictures | A page and an embedded image (`![alt](C:\path\image.png)`); clicking a picture |
| `clip.rs`, `keys.rs` | Ctrl+V in **New Thread** too; Ctrl+V outside the box pastes nothing |
| `UI.lua`, `Theme.lua` | All three themes and **Compact thread list** in Settings; `npm run screenshots` and compare |
| `.toc`, interface version | Tray **Match WoW's interface version…**, then the AddOns list shows nothing out of date |
| `tray.rs`, `dialog.rs`, `app.rs` | Every menu item the change touched, and its notification |
| `updater.rs` | From the previous release's exe, **Check for updates** and **Update to x.y.z** (debug builds take `T3WOW_UPDATE_REPO` to update from another repository); the restarted companion says its new version |
| `config.rs`, Trust | **Commands from the game** set to **Read-only**: the window greys out Send, Stop and Settle and says why |
| Sign-in, T3 server | **Sign out of T3**, **Sign in to T3…**; the game drops the previous sign-in's threads |

If a step fails, `/t3 trace` and the tray's **Open log** have what a bug report needs.
