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
| Copy a screenshot (Win+Shift+S), press Ctrl+V in the box by hand, send | Thumbnail above the box; the image in T3's message | clipboard, Ctrl+V hook, upload |
| Ask an agent to show a small chart with `html_render` | It shows in the conversation as a picture | Edge drawing, image slots |
| Let a thread you're not viewing finish a run of half a minute or more (ask it to run `sleep 30`, then reply) | Notification slides in, chat line, sound | alerts |
| `/reload` | Connects again; no notification sound for old news | new UI session |

Steps that check notifications need runs long enough for the companion to see the thread working. A thread that finishes in a few seconds, before the companion's next look, was never news to it: it doesn't notify. That's by design, not a failure.

Paste checks need a real Ctrl+V (Cmd+V on a Mac), held as a person presses it. On a Mac the paste guard sees Cmd+V only while it's held through one of its checks, so a key press an automation tool sends in an instant is refused like an addon's paste, by design. Press the keys yourself, on Windows too.

## More, for what the change touched

| Changed | Also check |
| --- | --- |
| `capture.rs`, `codec.rs`, `Codec.lua` | Long messages, an answer to a question, Stop: commands of every size get through |
| `slots.rs`, `Link.lua`, signals | `/t3 probe`; a long session (`/t3 status` counts the 200 updates); WoW running when the addon files change: the window's top line says to restart WoW; the AddOns list in the game shows T3 WoW as one entry, with no slot rows under it |
| `render.rs`, pictures | A page and an embedded image (`![alt](C:\path\image.png)`); clicking a picture |
| `audio.rs`, video and audio | An embedded video (`![demo](C:\path\demo.mp4)`): its frame with a play mark, and a click opens it in the PC's player. An embedded `.mp3`: its card, a click plays it out loud, a second click stops it; opening another thread stops it too. On a Mac, an Ogg Opus file plays too, and a WebM audio file (`.weba`) says the Mac can't play it. On Windows, test with MP3 or WAV: a PC with a codec pack (K-Lite, say) plays Ogg, which a stock PC opens in the default player instead |
| `cites.rs`, quotes | A quote made in T3 with **Cite in composer**, sent: the game shows the quoted words beside a bar and your comment under them; clicking it scrolls to the response it quotes and colors the words for a moment. A quote of a message further back loads earlier messages; one from another thread opens that thread |
| `Link.lua`, notifications | With WoW closed, let two threads finish runs of half a minute or more, and settle one of them in T3. Start WoW: only the unsettled one notifies, and `/t3 status` lists the settled one as "not shown: settled since" |
| Copy icon (`clip.rs`, `UI.lua`) | Point at a message: its copy icon, top right; a click says **Copied**, and the clipboard holds the message's markdown. A code block's icon copies its code alone, without fences. Shift-click a message: it opens selected in a box, and Ctrl+C (Cmd+C on a Mac) copies it and closes the box. **Commands from the game** set to **Read-only** still copies |
| `clip.rs`, `keys.rs` | Ctrl+V in **New Thread** too; Ctrl+V outside the box pastes nothing; an image picked with **Win+V**; two image files (a PNG and a JPEG) copied in File Explorer, then Ctrl+V: two thumbnails |
| `UI.lua`, `Theme.lua` | All three themes and **Compact thread list** in Settings; **Fade window** set to **Dim**: the window dims while you run and in a fight (with no addon error), and comes back when you stop, leave combat or point at it; dying closes it, and it opens again once you're alive; `npm run screenshots` and compare |
| `.toc`, interface version | Tray **Match WoW's interface version…**, then the AddOns list shows nothing out of date |
| `tray.rs`, `dialog.rs`, `app.rs` | Every menu item the change touched, and its notification |
| `updater.rs` | From the previous release's exe, **Check for updates** and **Update to x.y.z** (debug builds take `T3WOW_UPDATE_REPO` to update from another repository); the restarted companion says its new version |
| `config.rs`, Trust | **Commands from the game** set to **Read-only**: the window greys out Send, Stop and Settle and says why |
| Sign-in, T3 server | **Sign out of T3**, **Sign in to T3…**; the game drops the previous sign-in's threads |

If a step fails, `/t3 trace` and the tray's **Open log** have what a bug report needs.
