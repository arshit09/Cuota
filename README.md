# Cuota

A tiny Windows widget that shows your Claude plan usage: session (5hr), weekly, and per-model limits. Same numbers as Claude Code's **Account & Usage** panel.

Clawd, the Claude Code mascot, dances while Claude works and sleeps when it's idle.

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

**Double-click** the panel to expand it with usage details, and again to shrink it. In settings, **Double-click** can open your usage page on claude.ai or the Claude app instead. The details show:

- each limit's reset time and where it's headed at your current pace
- what your week went to: Claude Code, chats, Cowork
- your subscription and extra usage
- the Claude Code sessions running now
- Claude Code's replies, tool calls and tokens this week, by day, model and project, on this PC

**Show as** (pick one or more):

- **Popup**: floats above all windows
- **Desktop**: sits behind windows, like a desktop widget
- **Taskbar**: Clawd and each limit as tray icons. Click one for the panel.

Drag the panel to move it. Popup and Desktop are the same panel, so only one can be on.

**Display** sets the Panel and Taskbar separately: Bars or Pie, which limits, percentage, time left, and Clawd.

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
