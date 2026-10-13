# Cuota

A tiny Windows widget for your Claude plan usage: session (5hr), weekly, and per-model limits such as Weekly Fable. These are the same numbers as Claude Code's **Account & Usage** panel. It comes with Clawd, the Claude Code mascot, who dances while Claude is working and sleeps when it's idle.

## Requirements

- Windows 10/11
- [Claude Code](https://claude.com/claude-code) logged in with a Claude subscription (Pro or Max). Cuota reuses that login, so there's no separate sign-in.
- Rust (only needed to build it)

## Build & run

```powershell
cargo build --release
.\target\release\cuota.exe
```

The exe is self-contained, so you can copy `target\release\cuota.exe` anywhere. To start it with Windows, press `Win+R`, type `shell:startup`, and put a shortcut to `cuota.exe` in that folder.

## Using it

**Right-click** Cuota to open its settings. Under **Show as**, pick one:

- **Popup** floats above all windows. Drag it to move it.
- **Desktop** sits behind your windows, like a desktop widget. Drag it to move it.
- **Taskbar** shows icons beside the clock. Click an icon for the full panel, or right-click it for settings. Windows puts new icons in the **^** overflow at first, so drag them onto the taskbar once.

The **Panel** and **Taskbar** columns in the settings are set separately. In each one you can choose:

- which limits to show
- the percentage
- time left (on the taskbar it appears when you hover an icon)
- Clawd
- **Bars** or **Pie** style

Hover a meter to see its exact reset time. Meters turn amber at 75% and red at 90%. Turn off *Animate Clawd* to use almost no CPU.

Options: `--interval 120` sets how often usage refreshes while Claude is in use, in seconds (default 60, minimum 30). `--demo` shows fake data.

## How it works

Cuota reads the OAuth token that Claude Code saves in `%USERPROFILE%\.claude\.credentials.json`. It respects `CLAUDE_CONFIG_DIR` if you set one, and like Claude Code it uses `CLAUDE_CODE_OAUTH_TOKEN` instead when that's set. With the token it calls `https://api.anthropic.com/api/oauth/usage`, the same endpoint Claude Code uses. The token is sent only to Anthropic, and Cuota never writes to Claude's files.

To keep API calls rare, Cuota:

- reuses usage that Claude Code has already fetched
- calls the API about once a minute while a Claude Code session is working
- calls every 10 minutes when everything is idle
- calls right after a limit resets

Clawd's mood comes from the session status files Claude Code keeps in `%USERPROFILE%\.claude\sessions`. Cuota saves its own settings to `%APPDATA%\Cuota\settings.json`.

The usage endpoint is undocumented, so a future Claude update could change it.

## Troubleshooting

| Message | Fix |
|---|---|
| *Login token expired* | Claude Code refreshes the token when you use it. Open Claude Code and send a message, and Cuota picks up the new token within 15 s. |
| *No Claude login found* / *Login rejected* | Run `claude`, then `/login`. |
| *Rate limited* | Cuota backs off automatically. Use a longer `--interval` if it keeps happening. |
| Taskbar icons don't show | Click **^** next to the clock and drag the Cuota icons onto the taskbar. |

`cargo run -- --once` prints your current usage (or the error) to the terminal.
