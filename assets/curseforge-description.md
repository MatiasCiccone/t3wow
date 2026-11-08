# T3 WoW

T3 WoW puts T3 Code inside your WoW: Forever client, so you can read and reply to your coding agents without alt-tabbing.

## Requires the T3 WoW companion

**This addon doesn't work on its own.** It needs the T3 WoW companion, t3wow.exe, running on the same Windows PC while you play. Addons can't talk to other programs, so the companion carries everything between the game and T3 Code. Without it, the window only says "Waiting for the companion…".

**Get it:** download t3wow.exe from https://github.com/MatiasCiccone/t3wow-releases/releases/latest and run it. It installs this addon for you and keeps both up to date. See Setting up below.

## What it does

**Your threads, as in T3.** The window lists your T3 Code threads from this PC and from any machines linked to it, grouped like T3's sidebar: Open, Working, Snoozed and Settled. Each title shows the machine and project under it. Threads an agent started appear indented under the thread that started them, as in T3. **Search**, over the list, narrows it as you type to threads whose title, project or machine matches; Enter opens the first one.

**Read and reply.** Pick a thread to read its conversation. Type in the box and press Enter to message it (Shift+Enter for a new line). If the thread is busy, your message steers it or queues, as in T3. If T3 refuses a message, it comes back to the box with the reason. **Stop** interrupts a run, and **Settle** / **Unsettle** moves a thread between Open and Settled. Code and tables show in a monospace font (JetBrains Mono, which ships with the addon), with a table's columns lined up. Pages an agent shows (charts, mockups) and images a message embeds from a file appear as pictures, drawn by the companion as T3 draws them; click one to enlarge it.

**Start threads.** **New Thread** asks for the machine, the project, and whether the thread runs in a new worktree or the current checkout, then starts a thread with your message as its task and opens it.

**Answer questions.** When an agent asks you something, the question shows at the end of the conversation. Pick an option, or several if the question allows it, and press **Send answer**. You can also type your own answer and press Enter. If an answer doesn't go through, you can send it again.

**Notifications.** When a thread finishes, fails or needs you, a notification slides in near the top of the screen, along with a line in chat and the same sound T3 Code plays. Click it to open the thread, or right-click to dismiss it.

**Approvals.** You can approve a command or an edit only in T3. By design, T3 doesn't take approvals from anywhere else. A thread waiting for one shows **Approve in T3**, and its **Open in T3** button opens T3 on that thread. **Stop** cancels the run from the game.

## Themes

The window comes in three themes with the same layout. Only the colors, fonts and corners change:

- **T3** (the default) looks like the T3 Code app: dark rounded cards, status pills, your messages in bubbles, and the Inter font, which ships with the addon.
- **Flat** is square and pixel-bordered, with the game's Arial Narrow. If you run ElvUI, it uses ElvUI's colors, font and pixel size, so the window matches the rest of your screen.
- **Classic** keeps the game's own feel: gold Friz Quadrata headings and grey bevelled edges.

**Compact thread list** fits each thread on one line, in any theme. Hover a thread to see its machine and project.

## Settings

Right-click the minimap button, type /t3 settings, or press **Settings** in the window.

- **Theme** picks T3, Flat or Classic, and **Compact thread list** fits each thread on one line. Both apply at once.
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
- T3 Code desktop running on the same PC, or on another machine this PC can reach over the network (run t3wow.exe login --origin http://<host>:<port> once to point the companion at it). Threads on other machines also appear when you link those machines to T3.
- The T3 WoW companion, t3wow.exe, from https://github.com/MatiasCiccone/t3wow-releases.

## Setting up

1. Download t3wow.exe and run it. It runs in the system tray. The app isn't signed, so Windows may warn about an unrecognized app the first time. Choose More info, then Run anyway.
2. It finds WoW: Forever, installs this addon and its update slots, and opens T3's sign-in page. Approve the sign-in, which lasts 30 days.
3. Restart WoW completely the first time (not /reload). In the AddOns list, keep T3 WoW and the grey T3 WoW slot entries enabled.
4. Leave the companion running while you play. Its tray menu shows the T3 and WoW status, and has sign in and out, Choose WoW folder, Reinstall addon, Match WoW's interface version, Commands from the game, Open log, Start with Windows and the update check.

The companion checks for a new version when it starts and every 6 hours, and updates itself from the tray menu. It installs an update only when the download matches the release's checksum (and, once release signing is set up, when the release is signed with the project's release key). The companion includes the addon, so the addon updates with it. If you install the addon from CurseForge, it updates here, and the companion never replaces a newer addon with an older one.

## Trust and streaming

Any addon you install can draw the same strip of squares T3 WoW draws, and the companion can't tell them apart. With the default setting, another addon could message your threads or start a new thread with any task, with the access you granted T3 when you signed in. The companion's tray menu has **Commands from the game**:

- **Full** (the default): everything the window offers.
- **No new threads**: everything but starting threads.
- **Read-only**: you can read threads, but nothing from the game changes T3 (no messages, starts, answers, Stop or Settle).

It applies at once. The window greys out what the setting blocks and says why. Install addons you trust, or pick a stricter setting.

If you stream or record your game: the strip encodes what you send from the game as plain data, not encrypted, so anyone with a recording of the top-left corner of the screen can decode your messages. Crop or cover that corner.

## How it works

Addons can't talk to other programs, so the companion connects the two. While a command is waiting, the addon draws it as a small strip of colored squares in the top-left corner of the screen. The companion reads that corner of the WoW window and sends answers back as files in the AddOns folder, which the game loads. The companion never sends input to the game or reads its memory, and the addon uses only the documented addon API.

Each UI session has 200 updates, and the companion only spends one when something you can see changed. From 190, the window and a chat line say to Reload UI soon. After 200, notifications stop until you Reload UI. While the companion isn't running, the addon checks for updates less and less often, so it doesn't use them up.

## Commands

- /t3 opens and closes the window.
- /t3 settings opens the settings.
- /t3 status prints the connection state in chat.
- /t3 minimap shows or hides the minimap button.
- /t3 probe tests which file checks work on your client. Include its output in a bug report.
- /t3 trace prints the addon's latest trace lines, for debugging.

## Troubleshooting

- If you see "Waiting for the companion…", check that T3 WoW is running in the tray, that the game is windowed, and that nothing covers the top-left corner. **Open log** in the tray menu shows what the companion sees.
- If you see "Slot addons blocked", enable the T3 WoW slot addons in the AddOns list. If a game patch changed the interface version, choose Match WoW's interface version in the companion's tray menu, then restart WoW.
- If a button is greyed out and the window says it's off, check Commands from the game in the companion's tray menu.

T3 WoW is MIT licensed.
