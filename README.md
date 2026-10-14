# Cuota

<p align="center">
  <img src="docs/screenshots/clawd-at-work.png" width="383" alt="Cuota's panel: Clawd acts out one activity after another, with a word under him like Percolating…, beside bars for the Session, Weekly and Fable limits">
</p>

A tiny Windows widget that shows your Claude plan usage: session (5hr), weekly, and per-model limits. Same numbers as Claude Code's **Account & Usage** panel.

Clawd, the Claude Code mascot, acts out a random activity while Claude works: a new one whenever a session starts working, and another every so often. He greets you as work starts and celebrates when it's done, waits around for a minute after, then falls asleep until there's work again. Above the meters, a line says what he's up to, like Claude Code's spinner: a turning star, then *Percolating…*, *Spelunking…*, *Flibbertigibbeting…*, each new word typing itself over the last. The 62 animations come from [Mascot Studio](https://github.com/Harriszed/mascot-studio).

## Screenshots

**Clawd's moods**: working, waiting for a minute after a session goes idle, asleep once nothing is happening, and celebrating when work is done.

<img src="docs/screenshots/moods.png" width="776" alt="Four panels: Clawd running commands in sunglasses (Wrangling…), drinking coffee while he waits (Lollygagging…), asleep in a nightcap (Snoozing…), and celebrating under confetti (Nailed it!)">

**Make it yours**: rings instead of bars, Clawd without his status word, or just the meters.

<p>
  <img src="docs/screenshots/panel-pie.png" width="286" alt="The panel with a ring per limit, showing its percentage inside and the time left underneath">
  <img src="docs/screenshots/panel-no-status.png" width="383" alt="The panel with Clawd searching the web and no status word under him">
  <img src="docs/screenshots/panel-no-clawd.png" width="259" alt="The panel with only the limit bars, percentages and time left">
</p>

**Usage details and settings**: double-click the panel for details, right-click it for settings.

<table>
  <tr>
    <td valign="top"><img src="docs/screenshots/panel-details.png" width="383" alt="The expanded panel: each limit's reset time and pace, and what the week's usage went to by app"></td>
    <td valign="top"><img src="docs/screenshots/settings.png" width="340" alt="Cuota's settings: the account, Show as, Double-click, Start with Windows, Opacity and Animate Clawd, then what the panel displays"></td>
  </tr>
</table>

**Taskbar and tray menu**: with Show as set to Taskbar, Clawd dances by the clock while Claude works, and each limit gets an icon, as Bars or Pie (enlarged here). Right-click any of Cuota's icons for its menu.

<p>
  <img src="docs/screenshots/taskbar.png" width="208" alt="Taskbar icons: Clawd dancing, then 15, 49 and 92 percent as numbers over bars, and again inside rings">
  &nbsp;&nbsp;
  <img src="docs/screenshots/tray-menu.png" width="145" alt="The tray menu: Cuota v0.1.0 by Clawd, then Usage details, Settings and Quit">
</p>

The screenshots use `--demo`'s made-up account and usage.

## Requirements

- Windows 10/11
- [Claude Code](https://claude.com/claude-code), logged in with Pro or Max (Cuota reuses that login)
- Rust, to build

## Build & run

```powershell
cargo build --release
.\target\release\cuota.exe
```

The exe is self-contained, so copy it anywhere. To start it when you sign in, turn on **Start with Windows** in its settings. Only one Cuota runs at a time: launching it again opens the running one's settings.

| Flag | Does |
|---|---|
| `--interval 120` | Refresh every 120 s while Claude is in use (default 60, min 30) |
| `--demo` | Show fake data |
| `--once` | Print usage to the terminal and exit |

## Using it

**Right-click** Cuota for settings. It always has an icon in the tray by the clock, whose right-click menu has **Usage details**, **Settings** and **Quit**.

**Double-click** the panel to expand it with usage details, and again to shrink it. In settings, **Double-click** can instead open your usage page on claude.ai, the Claude app, or any file, program or script you pick. The details show:

- each limit's reset time and where it's headed at your current pace
- what your week went to: Claude Code, chats, Cowork
- your subscription and extra usage
- the Claude Code sessions running now
- Claude Code's replies, tool calls and tokens this week, by day, model and project, on this PC

**Show as** (pick one or more):

- **Always on top**: floats above all windows
- **Desktop**: sits behind windows, like a desktop widget
- **Taskbar**: Clawd and each limit as tray icons. Click one for the panel.

Drag the panel to move it. Always on top and Desktop are the same panel, so only one can be on.

Settings has a page for each part, picked in its sidebar: **General** (Show as, Double-click, Start with Windows), **Appearance** (the panel's size and opacity, and Clawd's animation), and **Panel** and **Taskbar**, which each set what that one shows: Bars or Pie, which limits, percentage, time left, and Clawd, with or without his status line above the meters in the panel.

Tips:

- Hover a meter for its exact reset time.
- Meters turn amber at 75% and red at 90%.
- Turn off *Animate Clawd* to use almost no CPU.

## How it works

- Reads Claude Code's login token from `%USERPROFILE%\.claude\.credentials.json` (or `CLAUDE_CONFIG_DIR`, or `CLAUDE_CODE_OAUTH_TOKEN`).
- Calls `api.anthropic.com/api/oauth/usage`, the same endpoint Claude Code uses. The token goes only to Anthropic.
- For usage details, reads Claude Code's session transcripts in `%USERPROFILE%\.claude\projects`, only while the panel is expanded. They stay on your PC.
- Never writes to Claude's files. Its own settings live in `%APPDATA%\Cuota\settings.json`.

API calls stay rare: about once a minute while Claude is working, every 10 minutes when idle, and right after a limit resets. It also reuses usage Claude Code already fetched.

The endpoint is undocumented, so a Claude update could break it.

## Troubleshooting

| Problem | Fix |
|---|---|
| *Login token expired* | Send a message in Claude Code. Cuota picks up the new token within 15 s. |
| *No Claude login found* / *Login rejected* | Run `claude`, then `/login`. |
| *Rate limited* | Cuota backs off on its own. Try a longer `--interval`. |
| Taskbar icons missing | Click **^** by the clock and drag the Cuota icons onto the taskbar. |

## License

Cuota's code is free software, licensed under the [GNU General Public License v3.0](LICENSE) or any later version. You can use it, change it, share it and even sell it. If you share Cuota or anything built from its code, changed or not, you have to share the source code too, under the same license.

The license covers Cuota's own code only. It doesn't cover Clawd's animations in `assets/mascot`, Anthropic's names, logo or mascot, or the screenshots in `docs/screenshots`, which show them. See [Credits and legal notice](#credits-and-legal-notice).

## Credits and legal notice

Cuota is an unofficial project. It isn't made, endorsed or supported by Anthropic, and has no affiliation with Anthropic.

**Anthropic's names, logo and mascot.** Claude, Claude Code and Anthropic are trademarks of Anthropic PBC. The Claude logo and Clawd, the Claude Code mascot, are Anthropic's too. Cuota and its author don't own any of them. Cuota uses the names only to say which product it works with, and Anthropic hasn't reviewed or approved Cuota or how it uses any of them.

- The Claude spark (Cuota's window icon and the logo in the panel) and the tray's pixel-art Clawd are drawn from `claude-logo.svg` and `clawd.svg`, which ship with Anthropic's Claude Code extension for VS Code. Claude Code is © Anthropic PBC, all rights reserved, and its use is subject to [Anthropic's Commercial Terms of Service](https://www.anthropic.com/legal/commercial-terms).
- See Anthropic's [trademark guidelines](https://www.anthropic.com/legal/trademark-guidelines) before you use any of these in your own work.

**Clawd's animations.** The 62 GIFs in [assets/mascot](assets/mascot) come from [Mascot Studio](https://github.com/Harriszed/mascot-studio), an unofficial community project by Harriszed, built on [Ayotomiwa Wale-Durojaye's reference animation](https://ayotomcs.me/claude-mascot). Neither grants a license to the artwork. As of 2026-10-14:

- Mascot Studio has no license. Its notice, copied unchanged in [assets/mascot/NOTICE.md](assets/mascot/NOTICE.md), says it grants no open-source or commercial-use license and that permission to redistribute the artwork is unverified.
- The reference animation's page says "© Copyright 2025" and gives no permission to reuse it.
- The character they draw is Anthropic's.

The GIFs are here, with credit, for personal, non-commercial use. Being in this repo gives you no rights to them, and Cuota's license doesn't apply to them. They're built into the exe (about 3.5 MB of it), so the same goes for any `cuota.exe` you build. Get permission from their owners before you reuse them, share them, or share a build that includes them.

**Removal.** If you own any of this and want it taken down, [open an issue](https://github.com/arshit09/Cuota/issues) and it will be removed.

**No warranty.** Cuota is provided as is, without warranty of any kind. It uses your Claude Code login and an undocumented Anthropic endpoint, so use it at your own risk.
