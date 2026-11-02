# ccw — Claude Code multi-account launcher

[![crates.io](https://img.shields.io/crates/v/ccw.svg)](https://crates.io/crates/ccw)

Run several Claude Code accounts side by side and pick one from a TUI on startup.

Claude Code stores its auth and settings in `~/.claude` + `~/.claude.json`. Point `CLAUDE_CONFIG_DIR`
somewhere else and you get a fully independent account: its own login, projects, sessions, skills,
plugins and MCP servers. `ccw` manages one directory per account and gives you a picker.

```
  ccw — choose a Claude Code account

   ❯ 1  default        you@example.com (max)      2m ago
     2  work           work@company.com (max)     1h ago
     3  personal       logged out  personal

  ↑↓/jk move · enter launch · l login · a add · e label · r refresh · q quit   YOLO
```

A single static binary. No Node.js needed — only the `claude` CLI (Claude Code 2.x).

## Install

```bash
cargo install ccw
```

Or from source:

```bash
git clone https://github.com/eiei114/ccw.git
cd ccw
cargo install --path .
```

The binary lands in `~/.cargo/bin` (make sure it is on PATH).

**macOS note:** Claude Code keeps credentials in Keychain there, so `CLAUDE_CONFIG_DIR` alone does
not isolate logins. Use `CLAUDE_CODE_OAUTH_TOKEN` (see below) or the API key on macOS.

## Usage

```bash
ccw                     # TUI picker, then launch claude
ccw work                # launch a profile directly, no TUI
ccw work -- -p "hi"     # forward args to claude (-- separates ccw from claude args)
ccw --safe              # TUI, but do not add --dangerously-skip-permissions
```

Never opened the picker before? `ccw` lists `default` (`~/.claude`) and nothing else. Press `a` to
create the next account.

### Commands

| Command | What it does |
| --- | --- |
| `ccw` | TUI picker → launch |
| `ccw list` | profiles with account name, email, plan, label, last used |
| `ccw account [name]` | one profile in detail (alias `whoami`) |
| `ccw add` | log in to a new account and name the profile after it |
| `ccw add <email>` | sign in as that email (pre-filled), profile named after it |
| `ccw add <name>` | create a profile with a fixed name and log in |
| `ccw label [name] [text]` | set the picker display name (`ccw label` lists, no text clears) |
| `ccw clone <src> <dst>` | copy auth + settings + workspace trust into a new profile |
| `ccw clone <src> <dst> --no-auth` | copy settings only, then log in separately |
| `ccw login <name>` | re-login an existing profile |
| `ccw login <email>` | re-login the profile that owns that email (or create one) |
| `ccw rename <old> <new>` | rename a profile directory (keeps the label) |
| `ccw trust [name] [dir]` | mark a directory trusted so `settings.json` permissions apply |
| `ccw help` / `ccw --version` | usage text / version |

### Account names vs. profile names

Two different things, on purpose:

- **profile name** — the directory name under `~/.ccw/`, used on the command line (`ccw work`) and
  in `CLAUDE_CONFIG_DIR`.
- **label** — what the picker shows. Defaults to the local part of the signed-in email
  (`account@example.com` → `account`), so a fresh `ccw add` already looks right without you
  choosing a name.

`ccw add` with no name reads the account after login and names the profile after it
(`account@example.com` → `account`). Give the email up front and it is pre-filled on Claude's
login page. Keep your own scheme with `ccw add <name>`, and override only the display text with
`ccw label`:

```bash
ccw add                          # → login, then profile "account", label "account"
ccw add account@example.com      # → pre-filled email, profile "account"
ccw add work                     # → profile "work", label "work"
ccw add work --email me@corp.com # → profile "work", pre-filled email
ccw login work --email me@corp.com
ccw login me@corp.com            # → finds the profile that owns that email
ccw label work account           # → profile "work", label "account"
ccw label work                   # → clear the label
ccw label                        # → list labels
```

`--console` (API billing) and `--sso` are passed through to `claude auth login`.

The pre-filled email is only a hint to the login page. If your browser is already signed in to a
different Claude account, the flow may finish as that account; `ccw` then prints
`warning: signed in as X, not Y`. Sign out in the browser (or use a private window) and run
`ccw login <name> --email <email>` again.

### TUI keys

| Key | Action |
| --- | --- |
| `↑` `↓` / `k` `j` | move |
| `Enter` | launch selected |
| `1`–`9` | launch that row immediately |
| `l` | log in / re-login the selected profile (asks for an email to pre-fill) |
| `e` | edit the label (display name) |
| `a` | add a profile: type a name, or an email to sign in as |
| `r` | refresh status |
| `q` / `Esc` / `Ctrl-C` | quit |

The cursor starts on the most recently used profile. When stdin is not a terminal, `ccw` falls
back to a numbered list read from stdin.

## YOLO mode

Every launch appends `--allow-dangerously-skip-permissions --dangerously-skip-permissions`.

- per call: `ccw --safe`, `ccw work --safe`, `ccw work -- --safe`
- permanently: `CCW_SAFE=1`
- the TUI footer shows `YOLO` or `skip-permissions OFF` so you always know which mode you are in

## Where profiles live

```
~/.claude          → profile "default" (your existing Claude Code setup)
~/.ccw/<name>/     → every other profile (CLAUDE_CONFIG_DIR)
~/.ccw/.labels/    → picker display names (one plain text file per profile)
```

Set `CCW_HOME` to keep profiles somewhere else. Each profile directory holds its own
`.claude.json`, `.credentials.json`, `settings.json`, `projects/`, `sessions/`, `skills/`,
`plugins/`.

## Manual setup (no ccw)

If you only want the isolation and not the picker:

```bash
export CLAUDE_CONFIG_DIR="$HOME/.ccw/work"
claude          # first run: onboarding → /login with the work account
```

For automation and CI, skip the browser flow entirely:

```bash
claude setup-token                                   # long-lived subscription token
CLAUDE_CONFIG_DIR="$HOME/.ccw/work" \
CLAUDE_CODE_OAUTH_TOKEN=sk-ant-oat01-... claude
```

`ANTHROPIC_API_KEY` works the same way for pay-per-token accounts. Verify with
`CLAUDE_CONFIG_DIR=... claude auth status` — it prints `configDirectory`, `authMethod` and the email.

## Caveats

- Multiple accounts must each stay inside their own subscription terms. Sharing accounts or
  spreading usage to dodge rate limits is not what this is for.
- `~/.ccw/*` is outside your usual backup globs.
- New profiles report `this workspace has not been trusted`, which makes Claude Code ignore
  `permissions` from `settings.json`. Run `ccw trust <name>` (or accept the dialog once).
- `default` is reserved for `~/.claude` and cannot be renamed or created.

## Formerly ccx

This project was called `ccx` and was written in Node.js. It is now `ccw`, a Rust binary, and
profiles moved from `~/.claude-accounts` to `~/.ccw`. Move old profiles with
`mv ~/.claude-accounts ~/.ccw` (with every Claude Code session closed), or log in again.
`CCX_SAFE` is still honoured.

## License

MIT
