# Moving to a new T3 Code release

T3 WoW says which T3 Code it's made for: `t3Code` in `package.json` is the one place that version is set. The companion is built with it (it suggests updating a T3 that runs an older one), and `npm test` checks that the README and `assets/curseforge-description.md` say the same. Before that changes, compare what the new T3 changed with what the companion relies on: `npm run test:live` catches changed tool inputs and answers, but not a change in what the same inputs do.

## 1. Read the release notes

Read T3 Code's GitHub release notes for every tag since the one T3 WoW is made for. Most of the changes won't touch T3 WoW; look for these:

- **The tools the companion calls or reads** (`WATCHED_TOOLS` in `companion/src/bridge/tests/live.rs`): new or removed inputs, new values, and a different result from the same inputs. `html_render` isn't among them: the companion doesn't call it but reads its calls from the timeline, where its input becomes a picture, and since 0.1.10 T3 lists it only to agents in a thread (`PAGE_INPUTS` in the same file has the inputs the companion reads).
- **What the companion reads from a thread's timeline:** the item types (matched on `item["type"]` in `companion/src/bridge.rs`, e.g. `FoundWork::from_item`, `waiting_card` and `read_view_scan`), `html_render`'s input and `AskUserQuestion`'s output.
- **Sign-in:** the OAuth flow, scopes, access levels, and what the approval page grants by default.
- **Threads:** statuses, pending requests (questions, approvals, secrets), snooze and settle rules.
- **Providers and models:** new providers, model options and efforts.
- **What T3 now handles itself:** slash commands, send flags.
- **What the game copies from T3's window:** list rows, labels, ages.

Example: in 0.1.8, approving the sign-in with the code box left empty grants Read only. No schema changed, but a new sign-in can no longer send messages. Only the release notes show that.

## 2. Run the live tests

`npm run test:live` (see the README's **Live tests**) against the new T3:

- The tool check fails on any input added, removed or changed on a watched tool, or a watched tool T3 no longer offers, and names each one. It compares with `companion/tests/fixtures/t3-tools.json`, the tools as the companion was made for them.
- The page test has an agent show a page with `html_render` and fails if the call has an input the companion doesn't read (`PAGE_INPUTS`: a renamed one, or a new required one) or lacks `title`, `height` or `html`, then checks that the page reaches the game as a picture. T3's tool list can't show this tool's inputs, so a new optional input goes unnoticed until the agent uses it: look for one in the release notes.
- The shape check fails where T3 answers in a shape `FakeT3` doesn't know.
- The command tests and the replay fail where a command no longer works end to end.

## 3. Act on each change

For each change from step 1 or 2, fix the companion or the addon, or decide it needs nothing. Fixes come with unit tests against `FakeT3`: first make the fake answer as the new T3 does.

Then record the new T3's tools: `T3WOW_LIVE_UPDATE_TOOLS=1 npm run test:live`. If an input waits for a later fix, take it out of `t3-tools.json` by hand, and say so in its `t3` line, so the tool check keeps naming it until the fix is in.

## 4. Release

When `npm run test:live` passes, change `t3Code` in `package.json`, then the version in the README's **What you need** and in `assets/curseforge-description.md` (all four mentions), until `npm test` passes. Release as the README's **Releasing** says. If the companion or the addon changed, do the [smoke test in the game](smoke-test.md) first.
