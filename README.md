# ccx — Claude Code multi-account launcher

Run several Claude Code accounts side by side and pick one from a TUI on startup.

Claude Code stores its auth and settings in `~/.claude` + `~/.claude.json`. Point `CLAUDE_CONFIG_DIR`
somewhere else and you get a fully independent account: its own login, projects, sessions, skills,
plugins and MCP servers. `ccx` manages one directory per account and gives you a picker.

```
  ccx — choose a Claude Code account

   ❯ 1  default        you@example.com (max)      2m ago
     2  work           work@company.com (max)     1h ago
     3  personal       (not initialised — press a)

  ↑↓/jk move · enter launch · l login · a add · r refresh · q quit   YOLO
```

## Install

Requires Node.js 18+ and the `claude` CLI (Claude Code 2.x).

```bash
git clone https://github.com/eiei114/ccx.git
cd ccx
# put ccx.js + the shim for your shell somewhere on PATH
mkdir -p ~/.local/bin
cp ccx.js ccx ccx.cmd ~/.local/bin/
chmod +x ~/.local/bin/ccx ~/.local/bin/ccx.js
```

- **bash / zsh / Git Bash** → use the `ccx` shim
- **PowerShell / cmd / Windows Terminal** → use `ccx.cmd`
- **macOS note:** Claude Code keeps credentials in Keychain there, so `CLAUDE_CONFIG_DIR` alone does
  not isolate logins. Use `CLAUDE_CODE_OAUTH_TOKEN` (see below) or the API key on macOS.

## Usage

```bash
ccx                     # TUI picker, then launch claude
ccx work                # launch a profile directly, no TUI
ccx work -- -p "hi"     # forward args to claude (-- separates ccx from claude args)
ccx --safe              # TUI, but do not add --dangerously-skip-permissions
```

Never opened the picker before? `ccx` lists `default` (`~/.claude`) and nothing else. Press `a` to
create the next account.

### Commands

| Command | What it does |
| --- | --- |
| `ccx` | TUI picker → launch |
| `ccx list` | profiles with account name, email, plan, label, last used |
| `ccx account [name]` | one profile in detail (alias `whoami`) |
| `ccx add` | log in to a new account and name the profile after it |
| `ccx add <name>` | create a profile with a fixed name and log in |
| `ccx label [name] [text]` | set the picker display name (`ccx label` lists, no text clears) |
| `ccx clone <src> <dst>` | copy auth + settings + workspace trust into a new profile |
| `ccx clone <src> <dst> --no-auth` | copy settings only, then log in separately |
| `ccx login <name>` | re-login an existing profile |
| `ccx rename <old> <new>` | rename a profile directory (keeps the label) |
| `ccx trust [name] [dir]` | mark a directory trusted so `settings.json` permissions apply |
| `ccx help` | usage text |

### Account names vs. profile names

Two different things, on purpose:

- **profile name** — the directory name under `~/.claude-accounts/`, used on the command line
  (`ccx work`), in `CLAUDE_CONFIG_DIR`, and in this repo's docs.
- **label** — what the picker shows. Defaults to the local part of the signed-in email
  (`account@example.com` → `account`), so a fresh `ccx add` already looks right without you
  choosing a name.

Everyone's accounts differ, so `ccx add` with no name reads the account after login and names the
profile after it (`account@example.com` → `account`). Keep your own scheme with `ccx add <name>`, and
override only the display text with `ccx label`:

```bash
ccx add                    # → profile "account", label "account"
ccx add work               # → profile "work", label "work"
ccx label work account     # → profile "work", label "account"
ccx label work             # → clear the label
ccx label                  # → list labels
```

### TUI keys

| Key | Action |
| --- | --- |
| `↑` `↓` / `k` `j` | move |
| `Enter` | launch selected |
| `1`–`9` | launch that row immediately |
| `l` | log in / re-login the selected profile |
| `e` | edit the label (display name) |
| `a` | add a new profile |
| `r` | refresh status |
| `q` / `Esc` / `Ctrl-C` | quit |

The cursor starts on the most recently used profile.

## YOLO mode

Every launch appends `--allow-dangerously-skip-permissions --dangerously-skip-permissions`.

- per call: `ccx --safe`, `ccx work --safe`, `ccx work -- --safe`
- permanently: `CCX_SAFE=1`
- the TUI footer shows `YOLO` or `skip-permissions OFF` so you always know which mode you are in

## Where profiles live

```
~/.claude                    → profile "default" (your existing Claude Code setup)
~/.claude-accounts/<name>/   → every other profile (CLAUDE_CONFIG_DIR)
~/.claude-accounts/.labels/  → picker display names (one plain text file per profile)
```

Each profile directory holds its own `.claude.json`, `.credentials.json`, `settings.json`,
`projects/`, `sessions/`, `skills/`, `plugins/`.

## Manual setup (no ccx)

If you only want the isolation and not the picker:

```bash
export CLAUDE_CONFIG_DIR="$HOME/.claude-accounts/work"
claude          # first run: onboarding → /login with the work account
```

For automation and CI, skip the browser flow entirely:

```bash
claude setup-token                                   # long-lived subscription token
CLAUDE_CONFIG_DIR="$HOME/.claude-accounts/work" \
CLAUDE_CODE_OAUTH_TOKEN=sk-ant-oat01-... claude
```

`ANTHROPIC_API_KEY` works the same way for pay-per-token accounts. Verify with
`CLAUDE_CONFIG_DIR=... claude auth status` — it prints `configDirectory`, `authMethod` and the email.

## Caveats

- Multiple accounts must each stay inside their own subscription terms. Sharing accounts or
  spreading usage to dodge rate limits is not what this is for.
- `~/.claude-accounts/*` is outside your usual backup globs.
- New profiles report `this workspace has not been trusted`, which makes Claude Code ignore
  `permissions` from `settings.json`. Run `ccx trust <name>` (or accept the dialog once).
- `default` is reserved for `~/.claude` and cannot be renamed or created.

## License

MIT
