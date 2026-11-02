# Setup prompt

Paste this into a coding agent (Claude Code, pi, Codex, Cursor, …) on a fresh machine to install
`ccw`, create your first extra profile, and wire up the shell. Fill the `<>` placeholders first.

---

Install and configure `ccw`, a Claude Code multi-account launcher, on this machine.

## 1. Install

Run `cargo install ccw` (needs a Rust toolchain from <https://rustup.rs>). Confirm
`~/.cargo/bin` is on PATH, then run `ccw list`.

Requirement: the `claude` CLI (Claude Code 2.x). Verify with `claude --version`.

## 2. Create one profile per account

For each account <ACCOUNT_EMAIL_1>, <ACCOUNT_EMAIL_2>, …:

```bash
ccw add <ACCOUNT_EMAIL_1>
ccw add <ACCOUNT_EMAIL_2>
```

Passing the email pre-fills Claude's login page. It is only a hint: if the browser is already
signed in to another Claude account, sign out there first (or use a private window). Finish the
OAuth prompt for each, then confirm:

```bash
ccw list                       # email + plan should match the account you just used
ccw <profile-name> -- -p "reply with exactly: OK"
```

If a profile must reuse the existing `~/.claude` login (same account, separate state) use
`ccw clone default <name>` instead of `ccw add`. If the email and profile name should differ, use
`ccw add <name> --email <ACCOUNT_EMAIL>` and `ccw label <name> <display-name>`.

When a profile reports `this workspace has not been trusted`, run `ccw trust <profile-name>` inside
the directory you work in.

## 3. Make launching convenient

Default expectation: every launch uses `--dangerously-skip-permissions`. ccw already adds it.
Confirm the TUI footer shows `YOLO`. Set `CCW_SAFE=1` in the shell if this machine must stay in
normal permission mode.

Optional: alias `claude` to `ccw` so the picker always appears.

- PowerShell (`$PROFILE`): `function claude { ccw @args }`
- bash/zsh: `alias claude=ccw`

## 4. Verify isolation

```bash
ccw <profile-a> -- -p "print the current model and nothing else"
ccw <profile-b> -- -p "print the current model and nothing else"
```

Check that `~/.ccw/<name>/` exists for each non-default profile and that `ccw list` shows a
distinct email per profile.

## 5. Report back

Print the final `ccw list` output, the install location, and whether YOLO mode is on by default.
Do not print any tokens, `.credentials.json` contents, or email addresses that were not already
shown to me.
