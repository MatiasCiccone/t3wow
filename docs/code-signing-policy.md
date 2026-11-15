# Code signing policy

Free code signing provided by [SignPath.io](https://signpath.io), certificate by [SignPath Foundation](https://signpath.org).

This applies once SignPath Foundation approves the project. Until then, `t3wow.exe` is not Authenticode-signed, and Windows may warn about an unrecognized app the first time it runs (see the README's [Set up](../README.md#set-up)). How releases are signed today, and how updates are checked, is in [release-signing.md](release-signing.md).

## What gets signed

`t3wow.exe`, the Windows companion, as built by the Release workflow (`.github/workflows/release.yml`) on a GitHub-hosted runner from a version tag in [MatiasCiccone/t3wow](https://github.com/MatiasCiccone/t3wow). Nothing built anywhere else, and nothing from another project, is signed with this certificate.

The Mac app (`T3 WoW.app`) is signed separately, with the project's own certificate ([The Mac app's certificate](release-signing.md#the-mac-apps-certificate)), and every release's checksum files are signed with the project's release key, which the companion's updater checks.

## Team roles and their members

- Committers and reviewers: the members of the [MatiasCiccone/t3wow](https://github.com/MatiasCiccone/t3wow) repository.
- Approvers: the owner, [MatiasCiccone](https://github.com/MatiasCiccone).

Every release is approved by hand before it is signed. Everyone in these roles uses multi-factor authentication on GitHub and on SignPath.

## Privacy policy

This program will not transfer any information to other networked systems unless specifically requested by the user or the person installing or operating it, except as follows:

- **The T3 Code server you sign in to.** By default that's T3 Code running on the same computer; you can point the companion at T3 Code on another machine by entering its address. The companion signs in to it (T3's sign-in page opens in your browser, and the companion receives the sign-in on a port on 127.0.0.1 that only this computer can reach), reads your threads from it, and sends it what you do in the game's T3 WoW window: messages, answers, new threads, stops, images you paste. That's the purpose of the program.
- **GitHub, to check for and download updates.** The companion asks GitHub (`api.github.com`, or the `github.com` releases page when the API limits it) for this project's latest release, and downloads a new release's files from GitHub when you choose to update. These requests carry the companion's version in their User-Agent (`t3wow-companion/<version>`) and nothing about you or your threads. By default the companion checks when it starts and every 6 hours; the tray menu's **Update checks** changes that, and **Only when I check (no automatic checks)** turns automatic checks off, so it contacts GitHub only when you choose **Check for updates**.
- **Pages an agent showed.** When you open, in the game, a thread with a page an agent displayed through T3 Code (T3's `html_render`), the companion draws that page with a headless Microsoft Edge or Chrome, as T3 Code shows it in its own window. The page's scripts run for a few seconds, and images, fonts and scripts the page names on the web are loaded, by that browser, from wherever the page says. The companion sends the page nothing else, but the browser itself may contact its maker's services as it usually does.
- **Links you click.** A pull request's link that you open from the game opens in your browser; **Open in T3** hands T3 Code its own `t3code://` link, which stays on your computer.

Nothing else is sent anywhere: no analytics, no crash reports, no usage data.
