# T3 WoW

T3 WoW puts T3 Code inside your WoW: Forever client, so you can read and reply to your coding agents without alt-tabbing.

## Requires the T3 WoW companion

**This addon doesn't work on its own.** It needs the T3 WoW companion, t3wow.exe, running on the same Windows PC while you play. Addons can't talk to other programs, so the companion carries everything between the game and T3 Code. Without it, the window only says "Waiting for the companion…".

**Get it:** download t3wow.exe from https://github.com/MatiasCiccone/t3wow/releases/latest and run it. It installs this addon for you and keeps both up to date. See Setting up below.

**On a Mac:** the companion is the T3 WoW app, which lives in the menu bar. See On a Mac below.

## Made for T3 Code 0.1.11

**T3 WoW is developed and tested against the stable release of T3 Code, version 0.1.11.** On any other version, including pre-release builds, parts of it may not work.

## Highlights

- **T3's layout, in the game.** The thread list, message box, questions and status pills follow T3's own client.
- **Paste a screenshot to your agent.** Copy an image, press Ctrl+V in the message box, and it goes with your next message. Click it to see it bigger, before or after you send it.
- **Switch models without leaving the game.** Pick the model and effort right in the message box, and move a thread to another provider mid-conversation.
- **Launch agents between pulls.** Start a thread on any linked machine, with its project, model, effort, permissions and a worktree of its own.
- **Commands, skills and machines as you type.** Start a word with / or $ or @ and pick from the list.
- **Never miss a finished run.** A notification slides in with T3 Code's own sound when a thread finishes, fails or needs you. Click it to open that thread.
- **See what your agents did and made.** Their work between messages shows as collapsed rows, the files a turn changed are listed under it, and the charts and mockups they show appear as pictures right in the conversation.
- **Play the video and audio they embed.** A video shows its first frame and opens in your player with a click. An audio clip plays on your PC with a click and stops with another.
- **Copy in one click.** A copy icon on each message and code block puts it on your clipboard as T3 has it, emoji and all.
- **Keep an eye on your pull requests.** Each thread shows its linked PR, colored by whether it's open, merged or closed. Click one to open it in your browser.

## What it does

**Your threads, as in T3.** The window lists your T3 Code threads from this PC and from any machines linked to it, grouped like T3's sidebar. Open runs down from the top, and Working, Snoozed and Settled sit at the bottom and grow upward as you open them. Each row shows the project, the title and the machine. Threads an agent started appear indented under the thread that started them. **Search**, over the list, narrows it as you type to threads whose title, project or machine matches; Enter opens the first one. The mouse's side buttons go back and forward through the threads you've opened.

**What a row tells you.** The top right shows the thread's status while it has one: Working with how long it has run, Waiting, Input, Approval, Failed, Woke when it came back from a snooze, or Done when it finished and you haven't opened it in the game. Otherwise it shows the thread's age. Across from the machine is its pull request, green when open, purple when merged and red when closed; click it to open it in your browser, or hover +2 and the like to pick from several. Hover a row for **Settle** (**Unsettle** on a settled thread) and **Snooze**, which offers T3's snooze times and **Custom…** for a time you type, such as `45m`, `5pm`, `tomorrow 9am` or `fri 3pm`. On a snoozed thread the clock wakes it. A thread that needs you offers only **Settle**, and a settled one only **Unsettle**.

**Read and reply.** Pick a thread to read its conversation. Its header shows the branch its checkout is on and lists its pull requests, and a message that names one (#123) shows it in the same colors, to click. Type in the box and press Enter to message it (Shift+Enter for a new line). The box keeps the keyboard so you can type the next message; Escape gives it back to the game. If the thread is busy, your message steers it or queues, as in T3. If T3 refuses a message, it comes back to the box with the reason. During a run, the box's button is **Stop**, and it becomes **Send** as soon as you type or paste something.

**See the work.** What an agent did between its messages (thinking, tool calls, questions you answered) shows as dimmed rows, collapsed as in T3; click one to open it. A finished turn folds under **Worked for**, leaving its last message in view. A turn that changed files ends with a card listing them, as T3 does: the lines added and removed, then each file; click it to copy the list. Code and tables show in a monospace font (JetBrains Mono, which ships with the addon), with a table's columns lined up. Pages an agent shows (charts, mockups) and images a message embeds from a file appear as pictures, drawn by the companion as T3 draws them; click one to enlarge it. Emoji and marks the game's fonts can't draw show as the nearest ones they can (✅ as ✓, ❌ as ✗, ➜ as →) instead of empty boxes.

**Video and audio.** Video and audio a message embeds from a file on this PC show where T3 shows its players. WoW can't play either, so the companion does it outside the game. A video shows as the frame T3 shows before it plays, with a play mark and its length; click it to open it in your default video player. Audio shows as a small card with its file's name: click it to play it on your PC, at the computer's volume rather than the game's, and click again to stop. Opening another thread or quitting WoW stops it too. Windows plays MP3 and WAV, and other formats as far as your codecs go; a file it can't play opens in your default player instead. On a Mac, audio plays with macOS's own player, which takes MP3, WAV, AIFF, AAC, M4A and FLAC.

**Quotes.** A quote you made in T3 with **Cite in composer** shows in your message as a quote: the quoted words beside a bar, your comment under them. Click it to go to the response it quotes: the conversation scrolls there and the quoted words light up for a moment, as in T3. A quote from another thread opens that thread.

**Copy.** Put the pointer on a message and a copy icon shows at its top right, as in T3, with one on each code block. Click it to put the message on your clipboard as T3 has it (its markdown, emoji included), or the block's code alone. Right-clicking a message offers **Copy** too. To copy part of a message, Shift-click it, or right-click and pick **Select text…**: it opens in a box where Ctrl+C copies what you select (Cmd+C on a Mac). A plain click on a message does nothing.

**Paste images.** Copy an image (a screenshot, an image from a browser) and press Ctrl+V in the box: it shows small above the box, where a click enlarges it, and goes with your next message, in a thread or a new one. Once sent, it shows small under your message, and a click enlarges it there too. Images attached in T3 itself don't show in the game.

**Pick the model.** Inside the message box sit the model and its effort, as in T3. In an open thread, picking either changes it right away, on any provider the machine runs, and a move to another provider shows as a line in the conversation. A thread keeps the permissions it started with; to change them, use T3.

**Type less.** Start a word with / for the provider's commands and skills, $ for its skills, or @ for your other machines and models. The arrow keys move through the list, and Tab or Enter picks. Commands and skills are listed for the T3 running on this PC. A model picked with @ reads as its name in the box and names it for the agent, to delegate to it, say; it doesn't switch the thread's model. **/clear** starts a fresh session in the open thread, and a **New session** line marks it in the conversation, as in T3. **Scripts**, at the top right of a thread whose project has saved scripts in T3, lists them; picking one asks the thread's agent to run it.

**Start threads.** The pencil beside Search opens **New Thread** in the middle of the window, as in T3: pick the project, write your message, and choose the machine under the box. A project's thread also asks whether it runs in a new worktree or the current checkout. You can pick its model, effort and permissions too; whatever you leave alone stays T3's default.

**Answer questions.** When an agent asks you something, the question shows at the end of the conversation. Pick an option, or several if the question allows it, and press **Submit answer**. You can also type your own answer and press Enter; text you type with a picked option goes with it as a note. If an answer doesn't go through, you can send it again.

**Notifications.** When a thread finishes, fails or needs you, a notification slides in near the top of the screen, along with a line in chat and the same sound T3 Code plays. Click it to open the thread, or right-click to dismiss it. After a reload or a login, a thread still waiting on you gets a notification again, without the sound, unless its notification already ran its time or was clicked. A thread that finished, failed or needed you while the game was closed notifies when you're back, unless you settled it in the meantime.

**Approvals.** You can approve a command or an edit only in T3. By design, T3 doesn't take approvals from anywhere else. A thread waiting for one shows **Approve in T3**, and its **Open in T3** button opens T3 on that thread. **Stop** cancels the run from the game.

## Themes

The window comes in three themes with the same layout. Only the colors, fonts and corners change:

- **T3** (the default) looks like the T3 Code app: dark rounded cards, status pills, your messages in bubbles, and the Inter font, which ships with the addon.
- **Flat** is square and pixel-bordered, with the game's Arial Narrow. If you run ElvUI, it uses ElvUI's colors, font and pixel size, so the window matches the rest of your screen.
- **Classic** keeps the game's own feel: gold Friz Quadrata headings and grey bevelled edges.

**Compact thread list** fits each thread on one line, in any theme. Hover a thread to see its machine and project.

## Settings

Right-click the minimap button, type /t3 settings, or press the gear at the bottom left of the window.

- **Theme** picks T3, Flat or Classic, and **Compact thread list** fits each thread on one line. Both apply at once.
- **Fade window** dims the window (**Dim**) or makes it invisible (**Hide**) while you move, while you're in combat, or both, and brings it back when you stop. It starts at **Off**. It stays whole while you type in it or point at it.
- Turn the line in chat, the sound and the on-screen notification on or off.
- **Stay on screen** sets how long a notification stays up: 5, 8, 15 or 30 seconds, or until you click it.
- **At most** sets how many notifications stay on screen at once, from 1 to 10. The oldest one closes to make room, and a newer notification for the same thread replaces the old one.
- **Move notifications** lets you drag them where you want them, and **Test** shows one.
- Pick which outcomes notify: finishes, needs you, fails, and is stopped (that last one is off by default).
- **Clicking an approval opens T3** is off by default. While it's off, clicking an approval's notification opens the thread in the game, so a stray click while playing doesn't switch you out of the game.
- Show or hide the minimap button.

## Opening it

Type /t3, click the T3 WoW minimap button, or bind a key under Options → Keybindings → AddOns → T3 WoW. If another addon has loaded LibDBIcon, the button registers through it, so button collectors group it with your other buttons.

## What you need

- Windows, with WoW: Forever in windowed or borderless windowed mode (exclusive fullscreen blocks screen capture).
- T3 Code desktop 0.1.11, the stable release, running on the same PC or on another machine this PC can reach over the network (run t3wow.exe login --origin http://<host>:<port> once to point the companion at it). Threads on other machines also appear when you link those machines to T3.
- The T3 WoW companion, t3wow.exe, from https://github.com/MatiasCiccone/t3wow/releases/latest.

## Setting up

1. Download t3wow.exe and run it. It runs in the system tray. The app isn't signed, so Windows may warn about an unrecognized app the first time. Choose More info, then Run anyway.
2. It finds WoW: Forever, installs this addon and its update slots, and opens T3's sign-in page. Approve the sign-in, which lasts 30 days.
3. Restart WoW completely the first time (not /reload). Keep T3 WoW and its update slots enabled. In the game's AddOns list T3 WoW is one entry; the list at character select, where no addon runs yet, still shows the slots as grey T3 WoW slot rows.
4. Leave the companion running while you play. Its tray menu shows the T3 and WoW status, and has sign in and out, Choose WoW folder, Repair addon files, Put back missing addon files automatically, Reinstall addon, Match WoW's interface version, Commands from the game, Open log, Start with Windows and the update check.

The companion checks for a new version when it starts and every 6 hours, and updates itself from the tray menu. It installs an update only when the download matches the release's checksum (and, once release signing is set up, when the release is signed with the project's release key). The companion includes the addon, so the addon updates with it. If you install the addon from CurseForge, it updates here, and the companion never replaces a newer addon with an older one. Keep the two at the same version: when the companion is older than the addon, the window's top line reads **Update the T3 WoW companion**, and its tooltip and a chat line say how (Check for updates in the tray menu, then Update).

## On a Mac

On a Mac with Apple Silicon (tested on macOS 26.5.1 and 26.7.2), the companion is **T3 WoW.app**, for WoW: Forever's native Mac client. It lives in the menu bar instead of the tray, and its menu has the same items, with Start at login instead of Start with Windows. Fullscreen works: the app reads the game's own window. Keep WoW's Resolution at the display's native one (Render Scale can stay lower). Pasting images works with **Cmd+V** once T3 WoW has Input Monitoring: the first image paste asks for it; turn on T3 WoW in System Settings, Privacy & Security, Input Monitoring, then quit and reopen T3 WoW.

1. Download t3wow-macos-arm64.zip from the same releases page and double-click it. **In Finder, drag T3 WoW onto Applications**, then open it from there. (Opened anywhere else, it runs from a read-only copy that can't update itself.)
2. macOS says "T3 WoW" Not Opened. **Click Done**, not the highlighted Move to Trash, which deletes the app.
3. Open System Settings, then Privacy & Security, scroll to Security and click **Open Anyway**. Click **Open Anyway** again in the next dialog, and confirm with Touch ID or your password.
4. When macOS asks to let T3 WoW record the screen, click Open System Settings, turn on T3 WoW, then click **Quit & Reopen**. T3 WoW reads only the top-left corner of the game's window (or of the screen there, while its window capture starts up).
5. Sign in to T3 and restart WoW, as in Setting up.

Settings, sign-ins and the log are in ~/Library/Application Support/t3wow. The sign-ins are encrypted with the key in token.key beside them, so a copy of config.json alone means signing in again. Updates install from the menu bar, like on Windows, and keep the Screen Recording permission.

While the app reads the game, macOS counts that as sharing the screen and mutes notification banners. To see them, turn on System Settings, Notifications, **Allow notifications when mirroring or sharing the display**. Either way, the newest notice heads the T3 WoW menu for 10 minutes. Fullscreen WoW hides the menu bar: press Cmd+Tab to switch to another app, and the T3 icon shows over the game.

## Trust and streaming

Any addon you install can draw the same strip of squares T3 WoW draws. So T3 WoW signs every strip with a key the companion makes for your install, and the companion follows an unsigned strip only for reading. The first time, restart WoW once after the companion has installed the addon: until then the window says "Restart WoW to send from the game".

Signing stops another addon from drawing commands of its own, but not one written to work T3 WoW's own window from its code: what it sends that way is signed like your own messages. The companion's tray menu has **Commands from the game**:

- **Full**: everything the window offers, including starting threads with any task, with the access you granted T3 when you signed in. An install from an earlier release keeps this.
- **No new threads** (where a new install starts): everything but starting threads. Choose Full to start threads from the game.
- **Read-only**: you can read threads, but nothing from the game changes T3 (no messages, starts, answers, pasted images, model or effort changes, Scripts, Stop, Settle, Snooze or Wake). Copying and playing video or audio still work, since they change nothing in T3.

It applies at once. The window greys out what the setting blocks and says why. Install addons you trust, or pick a stricter setting.

If you stream or record your game: the strip encodes what you send from the game as plain data, not encrypted, so anyone with a recording of the top-left corner of the screen can decode your messages. Crop or cover that corner.

## How it works

Addons can't talk to other programs, so the companion connects the two. While a command is waiting, the addon draws it as a small strip of colored squares in the top-left corner of the screen. The companion reads that corner of the WoW window and sends answers back as files in the AddOns folder, which the game loads. To paste images, while the T3 WoW window is open and WoW is in front, the companion checks Ctrl+V (Cmd+V on a Mac) (those two keys only, about 60 times a second). It reads the clipboard only for a paste received within five seconds of such a press. The companion never sends input to the game or reads its memory, and the addon uses only the documented addon API.

Each UI session has 200 updates, and the companion only spends one when something you can see changed. From 190, the window and a chat line say to Reload UI soon. After 200, notifications stop until you Reload UI. Hover the pill beside the window's title for how many are left, how long ago the last one came, and the addon's and the companion's versions. While the companion isn't running, the addon checks for updates less and less often, so it doesn't use them up.

## Commands

- /t3 opens and closes the window.
- /t3 settings opens the settings.
- /t3 status prints the connection state in chat.
- /t3 minimap shows or hides the minimap button.
- /t3 probe tests which file checks work on your client. Include its output in a bug report.
- /t3 trace prints the addon's latest trace lines, for debugging.

## Troubleshooting

- If you see "Waiting for the companion…", check that T3 WoW is running in the tray, that the game is windowed, and that nothing covers the top-left corner. **Open log** in the tray menu shows what the companion sees.
- If you see "Slot addons blocked", enable the T3 WoW slot addons in the AddOns list at character select (the list in the game hides them). If a game patch changed the interface version, choose Match WoW's interface version in the companion's tray menu, then restart WoW.
- If a button is greyed out and the window says it's off, check Commands from the game in the companion's tray menu. If it says to restart WoW, restart it: the game loads the key that signs its commands only when it starts.
- If something behaves oddly after a T3 Code update, check your T3 Code version: T3 WoW is developed and tested against 0.1.11. When T3 Code is older, the companion's tray and the window's top line say to update it (in T3 Code, Settings > About > Check for Updates).

T3 WoW is MIT licensed, and its source is on GitHub: https://github.com/MatiasCiccone/t3wow. Bug reports go to its Issues there.
