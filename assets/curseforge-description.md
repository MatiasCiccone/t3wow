# T3 WoW

T3 WoW puts T3 Code inside your WoW: Forever client, so you can read and reply to your coding agents without alt-tabbing.

![An agent's question in the T3 WoW window](https://media.forgecdn.net/attachments/2159/877/question-png.png)

## What it does

**Your threads, as in T3.** The window lists your T3 Code threads from this PC and from any machines linked to it, grouped like T3's sidebar: Open, Working, Snoozed and Settled. Each title shows the machine and project under it. Threads an agent started appear indented under the thread that started them, as in T3.

![The T3 WoW window with the thread list and a conversation](https://media.forgecdn.net/attachments/2159/857/window-png.png)

**Read and reply.** Pick a thread to read its conversation. Type in the box and press Enter to message it (Shift+Enter for a new line). If the thread is busy, your message steers it or queues, as in T3. **Stop** interrupts a run, and **Settle** / **Unsettle** moves a thread between Open and Settled.

**Start threads.** **New Thread** asks for the machine and project, then starts a thread with your message as its task.

![Choosing the machine and project for a new thread](https://media.forgecdn.net/attachments/2159/886/new-thread-png.png)

**Answer questions.** When an agent asks you something, the question shows at the end of the conversation. Pick an option, or several if the question allows it, and press **Send answer**. You can also type your own answer and press Enter.

**Notifications.** When a thread finishes, fails or needs you, a notification slides in near the top of the screen, along with a line in chat and a sound. Click it to open the thread, or right-click to dismiss it.

![Two notifications: one thread needs you, another finished](https://media.forgecdn.net/attachments/2159/876/notifications-png.png)

**Approvals.** You can approve a command or an edit only in T3. By design, T3 doesn't take approvals from anywhere else. A thread waiting for one shows **Approve in T3**, and its **Open in T3** button opens T3 on that thread. **Stop** cancels the run from the game.

![A thread waiting for approval, with the Open in T3 button](https://media.forgecdn.net/attachments/2159/873/approval-png.png)

## Settings

Right-click the minimap button, type /t3 settings, or press **Settings** in the window.

- Turn the line in chat, the sound and the on-screen notification on or off.
- **Stay on screen** sets how long a notification stays up: 5, 8, 15 or 30 seconds, or until you click it.
- **At most** sets how many notifications stay on screen at once, from 1 to 10. The oldest one closes to make room, and a newer notification for the same thread replaces the old one.
- **Move notifications** lets you drag them where you want them, and **Test** shows one.
- Pick which outcomes notify: finishes, needs you, fails, and is stopped (that last one is off by default).
- **Clicking an approval opens T3** is off by default. While it's off, clicking an approval's notification opens the thread in the game, so a stray click while playing doesn't switch you out of the game.
- Show or hide the minimap button.

![The settings panel](https://media.forgecdn.net/attachments/2159/856/settings-png.png)

## Opening it

Type /t3, click the T3 WoW minimap button, or bind a key under Options → Keybindings → AddOns → T3 WoW. If another addon has loaded LibDBIcon, the button registers through it, so button collectors group it with your other buttons.

![The T3 WoW minimap button among other addon buttons](https://media.forgecdn.net/attachments/2159/874/minimap-png.png)

## What you need

- Windows, with WoW: Forever in windowed or borderless windowed mode (exclusive fullscreen blocks screen capture).
- T3 Code desktop running on the same PC. Threads on other machines appear when you link those machines to it in T3.
- The T3 WoW companion, t3wow.exe, from https://github.com/MatiasCiccone/t3wow-releases.

## Setting up

1. Download t3wow.exe and run it. It runs in the system tray. The app isn't signed, so Windows may warn about an unrecognized app the first time. Choose More info, then Run anyway.
2. It finds WoW: Forever, installs this addon and its update slots, and opens T3's sign-in page. Approve the sign-in, which lasts 30 days.
3. Restart WoW completely the first time (not /reload). In the AddOns list, keep T3 WoW and the grey T3 WoW slot entries enabled.
4. Leave the companion running while you play. Its tray menu shows the T3 and WoW status, and has sign in and out, Start with Windows, Reinstall addon, Open log and the update check.

The companion checks for a new version when it starts and every 6 hours, and updates itself from the tray menu. The companion includes the addon, so the addon updates with it. If you install the addon from CurseForge, it updates here, and the companion never replaces a newer addon with an older one.

## How it works

Addons can't talk to other programs, so the companion connects the two. While a command is waiting, the addon draws it as a small strip of colored squares in the top-left corner of the screen. The companion reads that corner of the WoW window and sends answers back as files in the AddOns folder, which the game loads. The companion never sends input to the game or reads its memory, and the addon uses only the documented addon API.

Each UI session has 200 updates. When they run out, the window offers Reload UI.

## Commands

- /t3 opens and closes the window.
- /t3 settings opens the settings.
- /t3 status prints the connection state in chat.
- /t3 minimap shows or hides the minimap button.

## Troubleshooting

- If you see "Waiting for the companion…", check that T3 WoW is running in the tray, that the game is windowed, and that nothing covers the top-left corner. **Open log** in the tray menu shows what the companion sees.
- If you see "Slot addons blocked", enable the T3 WoW slot addons in the AddOns list.

T3 WoW is MIT licensed.
