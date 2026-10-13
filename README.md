# Cuota

A small always-on-top Windows overlay that shows your Claude plan usage: session (5hr), weekly (7 day) and per-model weekly limits such as Weekly Fable. These are the same numbers as Claude Code's **Account & Usage** panel.

## Requirements

- Windows 10/11
- [Claude Code](https://claude.com/claude-code) logged in with a Claude subscription (Pro / Max / Team). Cuota reuses that login, so there's no separate sign-in.
- Rust (only needed to build it)

## Build & run

```powershell
cargo build --release
.\target\release\cuota.exe
```

The exe is self-contained, so you can copy `target\release\cuota.exe` anywhere.

**Start with Windows:** press `Win+R`, type `shell:startup`, and put a shortcut to `cuota.exe` in that folder.

## Using it

- **Drag** anywhere on the overlay to move it. Cuota remembers its position.
- **Right-click** to open the menu: *Refresh*, *Account* (shows email and organization), *Opacity*, *Quit*. The overlay has no taskbar button, so use *Quit* to close it.
- The bar turns amber at 75% and red at 90%.
- The status dot is green when the data is fresh, amber when the last refresh failed (older data is still shown), and red when there's no data.

Options: `cuota.exe --interval 120` sets the refresh interval in seconds (default 60, minimum 30). `cuota.exe --demo` shows the overlay with fake data.

## How it works

Cuota reads the OAuth token that Claude Code saves in `%USERPROFILE%\.claude\.credentials.json` (it respects `CLAUDE_CONFIG_DIR` if you set one, and like Claude Code it uses `CLAUDE_CODE_OAUTH_TOKEN` instead when that is set) and calls `https://api.anthropic.com/api/oauth/usage`, the same endpoint Claude Code uses. The token is sent only to Anthropic, and Cuota never writes to Claude's files. The endpoint is undocumented, so a future Claude update could change it.

## Troubleshooting

| Message | Fix |
|---|---|
| *Login token expired* | Claude Code refreshes the token when you use it. Open Claude Code and send a message, and Cuota picks up the new token within 15 s. |
| *No Claude login found* / *Login rejected* | Run `claude`, then `/login`. |
| *Rate limited* | Cuota backs off automatically. Use a longer `--interval` if it keeps happening. |

`cargo run -- --once` prints your current usage (or the error) to the terminal.
