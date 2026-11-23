# Smoke test in the game

`npm test` and `npm run test:live` cover the companion against T3 and the addon in a simulated game. What they can't reach is WoW itself and the computer it runs on: the screen strip, signal fonts and slot addons, pictures drawn with a browser, the clipboard and the paste key, sounds, the tray or menu bar and updates. This is the check for those, by hand, in about ten minutes.

**When:** before a release whose changes touch the companion or the addon (not for docs, tests or CI alone), and after a WoW patch. A T3 Code update alone needs only `npm run test:live`.

## Before you start

1. `npm test` and `npm run test:live` pass.
2. Build the companion and start it:
   - **Windows:** `npm run build:exe`, then quit the companion in the tray and start `dist\t3wow.exe`.
   - **Mac:** set up as the README's [On a Mac](../README.md#on-a-mac) says (WoW at the display's native resolution). `npm run build:app`, then quit T3 WoW from its menu bar icon and open `dist/T3 WoW.app`. Where a step says tray, use that menu bar menu (in fullscreen WoW, Cmd+Tab to reach it). A build signed ad hoc (no `T3WOW_CODESIGN_IDENTITY`) needs Screen Recording, and later Input Monitoring, turned on again: if the menu says it can't see the game, remove T3 WoW from Screen Recording with **−** and add `dist/T3 WoW.app` with **+**.
3. The WoW install is shared: another worktree's build may have replaced the addon, or still be running in the tray. The tray's **Open log** starts each run with `T3 WoW companion <version>`, and in the game `/dump C_AddOns.GetAddOnMetadata("T3WoW", "Version")` prints the addon's. Both must be the build under test.
4. Threads you start go in the **T3 WoW live tests** project; archive them in T3 when you're done.

## Core: every time

| Step | Expect | What it checks |
| --- | --- | --- |
| Start the companion | The menu's T3 line says connected | sign-in, addon files |
| Start WoW from scratch (not `/reload`) and log in | Chat says T3 WoW loaded, then connected to T3; the menu's WoW line says connected | finding WoW, slot addons, first update |
| `/t3 status` | **signals**, not timer | signal fonts |
| `/t3`, open a thread | The conversation shows within a second or two | strip read, updates arriving |
| Send a message to it | It shows in T3, and the reply in the game | the strip carrying commands |
| **New Thread** in the test project | It starts and opens in place of the composer | starting threads from the game |
| Copy a screenshot (Win+Shift+S; on a Mac Cmd+Ctrl+Shift+4), click in the box, press Ctrl+V (Cmd+V) yourself, send | Thumbnail above the box; the image in T3's message. On a Mac, the first paste may be refused with an offer to open Input Monitoring: turn on T3 WoW there, quit and reopen T3 WoW (the app, not the game), and paste again | clipboard, paste guard, upload |
| Ask an agent to show a small chart with `html_render` | It shows in the conversation as a picture | browser drawing (Edge or Chrome; on a Mac also Chromium), image slots |
| With another thread open in the window, have a thread run `sleep 30`, then reply; check its row in the list shows it working | Notification slides in, chat line, sound | alerts |
| `/reload` | Connects again; no notification sound for old news | new UI session |

**Notifications:** for a plain run like `sleep 30`, the companion knows it finished when one look at the thread list saw it working and a later one doesn't. It looks every 4 seconds or so with the T3 WoW window open and every 12 with it closed, plus however long a look takes. A run that starts and ends between two looks never showed as working, so it doesn't notify, by design. Half a minute is a starting point, not a rule: what matters is that the thread's row showed it working. In Settings, **Finishes**, **Line in chat**, **Sound** and **On-screen notification** must be on (they are by default).

**Pasting:** press the paste key by hand, with WoW in front and the message box focused. An image paste goes through only within 5 seconds of a Ctrl+V (Cmd+V) the companion saw in WoW, and it looks at the keys about 60 times a second. A Mac reports only the keys held at that moment, so a tap quicker than one look (as automation tools send) can be missed, and the paste is refused. Windows also reports a key pressed since the last look, so it catches quick presses, Win+V's included; only an instant automated chord (all four key events within microseconds) can be missed. Text paste is WoW's own and needs none of this.

## More, for what the change touched

| Changed | Also check |
| --- | --- |
| `capture.rs`, `codec.rs`, `Codec.lua` | Long messages, an answer to a question, Stop: commands of every size get through |
| `slots.rs`, `Link.lua`, signals | `/t3 probe`; a long session (`/t3 status` counts the 200 updates); WoW running when the addon files change: the window's top line says to restart WoW; the AddOns list in the game shows T3 WoW as one entry, with no slot rows under it |
| `render.rs`, pictures | A page and an embedded image (`![alt](C:\path\image.png)`, on a Mac `![alt](/Users/you/image.png)`); clicking a picture |
| `audio.rs`, video and audio | With an MP4 a browser plays and an MP3 or WAV of half a minute or more on this computer, ask an agent in a thread on its T3 to reply with `![demo](C:\Users\you\demo.mp4)` (on a Mac `/Users/you/demo.mp4`) and the same for the audio. Video: its frame with a play mark (given a supported browser and a free picture slot; otherwise a line naming it), and a click opens it in the computer's player. Audio: a click on its card plays it, a second click stops it; play it again and open another thread: it stops. Mac: an Ogg Opus file plays and a `.weba` says the Mac can't play it (seen on macOS 26; recheck on a new macOS). Windows: an Ogg file plays in the companion with a codec pack (K-Lite, say), or opens in the default player on a stock PC |
| `cites.rs`, quotes | In T3, select a plain-text phrase that occurs once in an agent's reply, **Cite in composer**, add a comment and send. The game shows the quoted words beside a bar and your comment under them; a click scrolls to the reply and colors the words for a moment. Cite a reply a page or two above what the game first shows: the click says it's loading earlier messages, then jumps there. Cite a reply in thread A and send it in thread B (both on machines the game lists): the click opens A |
| `Link.lua`, notifications | After the core steps, with T3 and the same companion left running: in two new test threads, start a `sleep 60` run each, check both rows show working in the game, then quit WoW. Let both finish, settle one in T3 (keep both listed, not archived), wait 30 seconds, then start WoW. Only the unsettled one notifies, and `/t3 status` lists the settled one's title with "not shown: settled since" |
| Copy icon (`clip.rs`, `UI.lua`) | `/t3 status` says commands are signed. On a reply with some Markdown (bold, a list) and a fenced code block: the message's copy icon, top right, says **Copied**, and pasted in a text editor it's the message's Markdown; the code block's icon copies its code alone, without fences. Shift-click the reply's text: it opens selected in a box, and Ctrl+C (Cmd+C) copies it and closes the box; pasted, it's the text as the game shows it. With **Commands from the game** set to **Read-only**, the icon still copies |
| `clip.rs`, `keys.rs` | Ctrl+V (Cmd+V) in **New Thread** too; outside the box it pastes nothing. Windows only: an image picked with **Win+V**; two image files (a PNG and a JPEG) copied in File Explorer, then Ctrl+V: two thumbnails. Mac only: a file copied in Finder pastes no image |
| `UI.lua`, `Theme.lua` | All three themes and **Compact thread list** in Settings; **Fade window** set to **Dim**: the window dims while you run and in a fight (with no addon error), and comes back when you stop, leave combat or point at it; dying closes it, and it opens again once you're alive; `npm run screenshots` and compare |
| `.toc`, interface version | Tray **Match WoW's interface version…**, then the AddOns list shows nothing out of date |
| `tray.rs`, `dialog.rs`, `app.rs` | Every menu item the change touched, and its notification |
| `updater.rs` | From the previous release's exe, **Check for updates** and **Update to x.y.z** (debug builds take `T3WOW_UPDATE_REPO` to update from another repository); the restarted companion says its new version |
| `config.rs`, Trust | **Commands from the game** set to **Read-only**: the window greys out Send, Stop and Settle and says why |
| Sign-in, T3 server | **Sign out of T3**, **Sign in to T3…**; the game drops the previous sign-in's threads |

If a step fails, `/t3 trace` and the tray's **Open log** have what a bug report needs.
