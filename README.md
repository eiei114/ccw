# ccw — Claude Code multi-account launcher

[![crates.io](https://img.shields.io/crates/v/ccw.svg)](https://crates.io/crates/ccw)
[![downloads](https://img.shields.io/crates/d/ccw.svg)](https://crates.io/crates/ccw)
[![ci](https://github.com/eiei114/ccw/actions/workflows/ci.yml/badge.svg)](https://github.com/eiei114/ccw/actions/workflows/ci.yml)
[![license](https://img.shields.io/crates/l/ccw.svg)](LICENSE)
[![msrv](https://img.shields.io/crates/msrv/ccw.svg)](https://crates.io/crates/ccw)

Run several Claude Code accounts side by side and pick one from a TUI on startup, together with the
model and effort level it should use.

Claude Code stores its auth and settings in `~/.claude` + `~/.claude.json`. Point `CLAUDE_CONFIG_DIR`
somewhere else and you get a fully independent account: its own login, projects, sessions, skills,
plugins and MCP servers. `ccw` manages one directory per account and gives you a picker.

```
  ccw — choose a Claude Code account  (5h / 7d window)

   ❯ 1  you        you@example.com     5h   8% · 7d  69%  ↻14m  max  just now  default
     2  work       work@company.com    5h 100% · 7d 100%  ↻6h4m  max  1h ago    work
     3  personal   logged out          usage n/a                        personal

  ↑↓/jk move · enter next · l login · a add · e label · r refresh · u usage · q quit   YOLO
```

The `5h / 7d` column is the account's rate-limit usage: the rolling 5-hour window and the
weekly window, each as a percentage of the plan allowance, and `↻` for the sooner of the two
reset times. It comes from Claude Code's own `GET /api/oauth/usage`, fetched in the background
so the picker never waits on the network.

Rows are laid out for the terminal they are drawn in and never wrap: a narrow window drops the
plan, then the age, then the email (whole, never as a fragment), then shortens the label, and
lastly the `↻` reset time. `COLUMNS` overrides the detected width, which also shapes piped
output.

`enter` moves on to the model, then the effort level, then launches:

```
  ccw — pick a model for work

   1  default       no --model, let Claude Code pick
 ❯ 2  opus          alias for the newest Opus
   3  sonnet        alias for the newest Sonnet
   4  haiku         alias for the newest Haiku
   5  fable         alias for the newest Fable

  ↑↓/jk move · enter next · c custom ID · esc back · q quit

  ccw — pick an effort level for work

   1  default       let Claude Code decide
   2  low           fastest answer, least thinking
   3  medium        less than the model default
 ❯ 4  high          the usual default
   5  xhigh         more thinking than default
   6  max           the most thinking the model allows

  ↑↓/jk move · enter launch · esc back · q quit
```

The choice is remembered per profile, so the second launch starts where you left off.

A single static binary. No Node.js needed — only the `claude` CLI (Claude Code 2.x).

Rust 1.80 or newer to build; the pre-built path needs only `cargo install`.

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
ccw                     # TUI picker → model → effort → launch claude
ccw work                # launch a profile directly, no TUI (remembered model/effort)
ccw work -- -p "hi"     # forward args to claude (-- separates ccw from claude args)
ccw --safe              # TUI, but do not add --dangerously-skip-permissions
ccw --model opus --effort high
                        # TUI, but skip the model and effort steps
ccw work --model sonnet --effort max
                        # no TUI, explicit model/effort (also remembered)
```

Never opened the picker before? `ccw` lists `default` (`~/.claude`) and nothing else. Press `a` to
create the next account.

### Commands

| Command | What it does |
| --- | --- |
| `ccw` | TUI picker → model → effort → launch |
| `ccw list` | profiles with account, plan, label, 5h/7d usage, last used, model, effort |
| `ccw account [name]` | one profile in detail (alias `whoami`) |
| `ccw defaults [name] [--model M] [--effort E]` | show or set the remembered launch choice (alias `model`) |
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

Account list:

| Key | Action |
| --- | --- |
| `↑` `↓` / `k` `j` | move |
| `Enter` | go to the model list |
| `1`–`9` | pick that row and go to the model list |
| `l` | log in / re-login the selected profile (asks for an email to pre-fill) |
| `e` | edit the label (display name) |
| `a` | add a profile: type a name, or an email to sign in as |
| `r` | refresh profiles and usage |
| `u` | re-fetch usage now (otherwise a snapshot younger than 60s is reused) |
| `q` / `Esc` / `Ctrl-C` | quit |

Model list:

| Key | Action |
| --- | --- |
| `↑` `↓` / `k` `j` | move |
| `Enter` / `1`–`9` | use that model, go to the effort list |
| `c` | type any alias or full model ID |
| `Esc` | back to the account list |

The cursor starts on the remembered model (or `default`).

Effort list:

| Key | Action |
| --- | --- |
| `↑` `↓` / `k` `j` | move |
| `Enter` / `1`–`9` | use that effort level and launch |
| `Esc` | back to the model list |

When stdin is not a terminal, `ccw` falls back to three numbered prompts read from stdin.

## Usage column

The `5h / 7d` column shows how much of each rate-limit window the account has used, and when the
sooner one resets. Numbers come from `https://api.anthropic.com/api/oauth/usage`, called with the
profile's OAuth token from `~/.ccw/<name>/.credentials.json`.

- `CCW_NO_USAGE=1` turns the network off; cached numbers are still shown (`usage off` when there
  are none).
- A snapshot younger than 60 seconds is reused; `u` in the picker forces a re-fetch.
- Expired or missing credentials show `usage n/a`. `ccw` does not refresh tokens — run
  `ccw login <name>` and the column fills in again.
- The endpoint rate-limits aggressive polling, and nothing here polls in the background or leaves
  a daemon behind.

## Model and effort

`ccw` passes `--model <value>` and `--effort <level>` to `claude` only when you pick something
other than `default`. Both are remembered per profile in `~/.ccw/.launch/<name>`.

| Flag | Values | What it does |
| --- | --- | --- |
| `--model <alias|id>` | `opus`, `sonnet`, `haiku`, `fable`, `default`, or any full Anthropic model ID | model for this session |
| `--effort <level>` | `low`, `medium`, `high`, `xhigh`, `max`, `default` | how much the model thinks per turn |
| `-Q`, `--quiet` | — | drop the `launching claude as …` line |

```bash
ccw defaults work --model opus --effort xhigh   # set once, no launch
ccw defaults work                               # what will launch
ccw defaults                                    # every profile
```

`--model` and `--effort` are consumed by `ccw` wherever they appear, except after `--`. Use
`ccw work -- --effort low` to hand a raw flag to `claude` instead. The flags write the remembered
choice too, so the picker opens on them next time.

Note that `ccw add` and `ccw login` ignore these two flags — they have no launch to attach them
to. Set the choice with `ccw defaults <name>` afterwards, or let the picker ask.

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
~/.ccw/.launch/    → remembered model/effort (one plain text file per profile)
~/.ccw/.usage/     → last known 5h/7d usage (one JSON file per profile)
```

Set `CCW_HOME` to keep profiles somewhere else. Each profile directory holds its own
`.claude.json`, `.credentials.json`, `settings.json`, `projects/`, `sessions/`, `skills/`,
`plugins/`.

## Environment

| Variable | Effect |
| --- | --- |
| `CCW_HOME` | where profiles live (default `~/.ccw`) |
| `CCW_SAFE=1` | never add `--dangerously-skip-permissions` |
| `CCW_NO_USAGE=1` | no network calls; the usage column shows cached numbers or `usage off` |
| `COLUMNS` | override the detected terminal width, which also shapes piped `ccw` output |
| `NO_COLOR` | plain output, no ANSI colours |
| `CCX_SAFE=1` | old name for `CCW_SAFE`, still honoured |

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
- The usage column reads a private endpoint (`/api/oauth/usage`). If Anthropic changes it, the
  column degrades to `usage n/a` and nothing else breaks.

## Changelog

### 0.5.0

- Picker rows are laid out for the actual terminal width and never wrap; columns are dropped
  cheapest-first as the window narrows.
- `ccw list` measures its own column widths, so the header lines up and model/effort no longer
  collapse into a single column.

### 0.4.1

- `ccw list` reports why usage is missing (`usage n/a`, `usage off`) instead of a bare `-`.

### 0.4.0

- Three-step picker: account → model → effort, with both choices remembered per profile.
- `5h / 7d` rate-limit usage column, fetched in the background and cached for 60s.
- `--model`, `--effort`, `-Q` flags and the `ccw defaults` command.
- `CCW_NO_USAGE=1` to keep every network call off.

### 0.3.0

- Rewritten in Rust (formerly `ccx`, a Node.js program).

## Formerly ccx

This project was called `ccx` and was written in Node.js. It is now `ccw`, a Rust binary, and
profiles moved from `~/.claude-accounts` to `~/.ccw`. Move old profiles with
`mv ~/.claude-accounts ~/.ccw` (with every Claude Code session closed), or log in again.
`CCX_SAFE` is still honoured.

## License

MIT
