# Setup prompt

Paste this into a coding agent (Claude Code, pi, Codex, Cursor, …) on a fresh machine to install
`ccx`, create your first extra profile, and wire up the shell. Fill the `<>` placeholders first.

---

Install and configure `ccx`, a Claude Code multi-account launcher, on this machine.

## 1. Install

Clone <https://github.com/eiei114/ccx> and put the launcher on PATH:

- bash/zsh/Git Bash: copy `ccx.js` and `ccx` (shim) to `~/.local/bin`, `chmod +x` both
- PowerShell/cmd/Windows: copy `ccx.js` and `ccx.cmd` to `%USERPROFILE%\.local\bin`
- verify `~/.local/bin` is on PATH, then run `ccx list`

Requirements: Node.js 18+ and the `claude` CLI (Claude Code 2.x). Verify with `node --version`,
`claude --version`.

## 2. Create one profile per account

For each account <ACCOUNT_EMAIL_1>, <ACCOUNT_EMAIL_2>, …:

```bash
ccx add <ACCOUNT_EMAIL_1>
ccx add <ACCOUNT_EMAIL_2>
```

Passing the email pre-fills Claude's login page, so the browser flow signs in as the intended
account. Finish the OAuth prompt for each, then confirm:

```bash
ccx list                       # email + plan should match the account you just used
ccx <profile-name> -- -p "reply with exactly: OK"
```

If a profile must reuse the existing `~/.claude` login (same account, separate state) use
`ccx clone default <name>` instead of `ccx add`. If the email and profile name should differ, use
`ccx add <name> --email <ACCOUNT_EMAIL>` and `ccx label <name> <display-name>`.

When a profile reports `this workspace has not been trusted`, run `ccx trust <profile-name>` inside
the directory you work in.

## 3. Make launching convenient

Default expectation: every launch uses `--dangerously-skip-permissions`. ccx already adds it.
Confirm the TUI footer shows `YOLO`. Set `CCX_SAFE=1` in the shell if this machine must stay in
normal permission mode.

Optional: alias `claude` to `ccx` so the picker always appears.

- PowerShell (`$PROFILE`): `function claude { ccx @args }`
- bash/zsh: `alias claude=ccx`

## 4. Verify isolation

```bash
ccx <profile-a> -- -p "print the current model and nothing else"
ccx <profile-b> -- -p "print the current model and nothing else"
```

Check that `~/.claude-accounts/<name>/` exists for each non-default profile and that
`ccx list` shows a distinct email per profile.

## 5. Report back

Print the final `ccx list` output, the PATH location used, and whether YOLO mode is on by default.
Do not print any tokens, `.credentials.json` contents, or email addresses that were not already
shown to me.
