#!/usr/bin/env node
/**
 * ccx - Claude Code multi-account launcher
 *
 * Usage:
 *   ccx                  TUI: pick an account, then launch claude
 *   ccx <name> [args...] launch claude with that profile (no TUI)
 *   ccx list             profiles with account, plan, label and last use
 *   ccx account [name]   one profile in detail (or "whoami")
 *   ccx label [name] [text]  set the picker display name (no text = clear, no name = list)
 *   ccx add <name>       create a profile and log in to it
 *   ccx clone <src> <dst> copy a profile (auth + settings) into a new one
 *   ccx login <name>     re-login an existing profile
 *   ccx rename <old> <new>  rename a profile directory
 *   ccx trust [name] [dir]  mark a directory trusted for that profile
 *   ccx -- <args...>     TUI, then pass args to claude (e.g. ccx -- -p "hi")
 *
 * Every launch adds --dangerously-skip-permissions (YOLO mode).
 * Disable per call with --safe, or globally with CCX_SAFE=1.
 *
 * Profiles live in %USERPROFILE%\.claude-accounts\<name> and are isolated by
 * CLAUDE_CONFIG_DIR. The pseudo profile "default" uses the normal ~/.claude.
 */
'use strict';

const fs = require('fs');
const path = require('path');
const os = require('os');
const readline = require('readline');
const { spawnSync } = require('child_process');

const HOME = os.homedir();
const ACCOUNTS_ROOT = path.join(HOME, '.claude-accounts');
const LABEL_DIR = path.join(ACCOUNTS_ROOT, '.labels');
const DEFAULT_DIR = path.join(HOME, '.claude');
const IS_WIN = process.platform === 'win32';
const C = process.env.NO_COLOR
  ? { r: '', g: '', y: '', b: '', d: '', c: '', x: '' }
  : {
      r: '\x1b[0m',
      g: '\x1b[32m',
      y: '\x1b[33m',
      b: '\x1b[36m',
      d: '\x1b[90m',
      c: '\x1b[1m',
      x: '\x1b[?25l',
    };
const SHOW_CURSOR = process.env.NO_COLOR ? '' : '\x1b[?25h';

// ------------------------------------------------------------------- yolo mode

const YOLO = '--dangerously-skip-permissions';
const YOLO_ARGS = ['--allow-dangerously-skip-permissions', YOLO];

function withYolo(args) {
  const safe = process.env.CCX_SAFE === '1' || process.env.CCX_SAFE === 'true';
  let rest = args.slice();
  if (rest[0] === '--') rest = rest.slice(1);
  const dropped = rest.filter((a) => a !== '--safe');
  const wantsSafe = dropped.length !== rest.length || safe;
  if (wantsSafe || dropped.includes(YOLO)) return dropped;
  return [...YOLO_ARGS, ...dropped];
}

// ---------------------------------------------------------------- executable

function findClaude() {
  const exts = IS_WIN ? ['.exe', '.cmd', '.bat', ''] : [''];
  const dirs = (process.env.PATH || '').split(path.delimiter).filter(Boolean);
  for (const dir of dirs) {
    for (const ext of exts) {
      const candidate = path.join(dir, 'claude' + ext);
      try {
        if (fs.statSync(candidate).isFile()) return candidate;
      } catch {}
    }
  }
  return 'claude';
}
const CLAUDE = findClaude();

function quoteWin(s) {
  if (!/[\s"]/.test(s)) return s;
  return '"' + s.replace(/"/g, '\\"') + '"';
}

// node >=20.12 refuses to spawn .cmd/.bat without a shell
function spawnClaude(args, opts) {
  if (IS_WIN && /\.(cmd|bat)$/i.test(CLAUDE)) {
    return spawnSync([CLAUDE, ...args].map(quoteWin).join(' '), { ...opts, shell: true });
  }
  return spawnSync(CLAUDE, args, opts);
}

function runClaude(args, env) {
  const res = spawnClaude(args, {
    stdio: 'inherit',
    env: { ...process.env, ...env },
    windowsHide: false,
  });
  return res.status == null ? 1 : res.status;
}

function captureClaude(args, env) {
  const res = spawnClaude(args, {
    encoding: 'utf8',
    timeout: 20000,
    env: { ...process.env, ...env },
    windowsHide: true,
  });
  return { out: (res.stdout || '') + (res.stderr || ''), status: res.status };
}

// ------------------------------------------------------------------- profiles

function profileDir(name) {
  return name === 'default' ? DEFAULT_DIR : path.join(ACCOUNTS_ROOT, name);
}

function profileEnv(name) {
  if (name === 'default') return {};
  return { CLAUDE_CONFIG_DIR: profileDir(name) };
}

function listProfileNames() {
  let names = [];
  try {
    names = fs
      .readdirSync(ACCOUNTS_ROOT, { withFileTypes: true })
      .filter((e) => e.isDirectory() && !e.name.startsWith('.'))
      .map((e) => e.name)
      .sort();
  } catch {}
  return ['default', ...names];
}

function slugify(value) {
  return String(value || '')
    .toLowerCase()
    .replace(/@.*$/, '')
    .replace(/[^a-z0-9._-]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 24);
}

// Display name written by the user (ccx label / TUI 'e' key).
function readLabel(name) {
  try {
    return fs.readFileSync(path.join(LABEL_DIR, name), 'utf8').trim() || null;
  } catch {
    return null;
  }
}

function writeLabel(name, label) {
  if (!label) {
    try {
      fs.unlinkSync(path.join(LABEL_DIR, name));
    } catch {}
    return;
  }
  fs.mkdirSync(LABEL_DIR, { recursive: true });
  fs.writeFileSync(path.join(LABEL_DIR, name), label + '\n');
}

function lastUsed(name) {
  const base = name === 'default' ? DEFAULT_DIR : profileDir(name);
  const candidates =
    name === 'default'
      ? [path.join(HOME, '.claude.json'), path.join(base, '.claude.json'), base]
      : [path.join(base, '.claude.json'), base];
  let best = 0;
  for (const c of candidates) {
    try {
      best = Math.max(best, fs.statSync(c).mtimeMs);
    } catch {}
  }
  return best;
}

function accountName(email) {
  if (!email) return null;
  const at = String(email).indexOf('@');
  return at > 0 ? String(email).slice(0, at) : String(email);
}

function describe(name) {
  return describeWithSummary(name, { skipCli: false });
}

// Read auth details from the profile's own cached files first, so the picker does not
// spawn a `claude` process per profile. Falls back to `claude auth status` when the
// cached data cannot answer (e.g. token-only auth, first run after /login).
function describeWithSummary(name, opts) {
  const dir = profileDir(name);
  const exists = name === 'default' ? fs.existsSync(DEFAULT_DIR) : fs.existsSync(dir);
  const info = {
    name,
    dir,
    exists,
    email: null,
    plan: null,
    auth: null,
    label: readLabel(name),
    lastUsed: lastUsed(name),
  };
  if (!exists) {
    info.auth = 'missing';
    return info;
  }

  const jsonPath =
    name === 'default' ? path.join(HOME, '.claude.json') : path.join(dir, '.claude.json');
  let parsed = null;
  try {
    parsed = JSON.parse(fs.readFileSync(jsonPath, 'utf8'));
  } catch {}

  const acct = parsed && parsed.oauthAccount;
  if (acct && acct.emailAddress) {
    info.email = acct.emailAddress;
    const fromOrg = String(acct.organizationType || '').replace(/^claude_/i, '');
    const fromTier = /(max|pro|team|enterprise|free)/i.exec(
      `${acct.organizationRateLimitTier || ''} ${acct.seatTier || ''}`
    );
    info.plan = fromOrg || (fromTier ? fromTier[1].toLowerCase() : null);
  }

  const credPath =
    name === 'default'
      ? path.join(DEFAULT_DIR, '.credentials.json')
      : path.join(dir, '.credentials.json');
  const hasCreds = fs.existsSync(credPath);
  if (info.email) info.auth = 'claude.ai';
  else if (hasCreds) info.auth = 'unknown';
  else info.auth = 'logged-out';

  const needCli =
    !opts.skipCli && (!info.email || info.auth === 'logged-out' || info.auth === 'unknown');
  if (needCli) {
    const { out } = captureClaude(['auth', 'status'], profileEnv(name));
    try {
      const json = JSON.parse(out.slice(out.indexOf('{')));
      info.auth = json.loggedIn ? json.authMethod : 'logged-out';
      if (json.email) info.email = json.email;
      if (json.subscriptionType) info.plan = json.subscriptionType;
    } catch {
      /* keep what we read from disk */
    }
  }
  return info;
}

function humanAge(ms) {
  if (!ms) return 'never';
  const diff = Date.now() - ms;
  const min = Math.round(diff / 60000);
  if (min < 1) return 'just now';
  if (min < 60) return `${min}m ago`;
  const h = Math.round(min / 60);
  if (h < 24) return `${h}h ago`;
  return `${Math.round(h / 24)}d ago`;
}

function profileLine(p) {
  const label = (p.label || (p.email && accountName(p.email)) || p.name).slice(0, 20).padEnd(20);
  const tail = p.name === 'default' ? `${C.d}default${C.r}` : `${C.d}${p.name}${C.r}`;
  if (!p.exists) return `${label} ${C.d}(new — press a)${C.r}`;
  if (p.auth === 'logged-out') return `${label} ${C.y}logged out${C.r}  ${tail}`;
  const who = p.email || p.auth || '?';
  const plan = p.plan ? ` ${C.d}(${p.plan})${C.r}` : '';
  const age = p.lastUsed ? ` ${C.d}${humanAge(p.lastUsed)}${C.r}` : '';
  const dirHint = p.label && p.label !== p.name ? ` ${tail}` : '';
  return `${label} ${C.b}${who}${C.r}${plan}${dirHint}${age}`;
}

// --------------------------------------------------------------------- actions

const EMAIL_RE = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

function loginArgs(email, opts) {
  const args = ['auth', 'login'];
  if (opts && opts.console) args.push('--console');
  if (opts && opts.sso) args.push('--sso');
  if (email) args.push('--email', email);
  return args;
}

// Profile name from an email: local part, slugified.
function profileNameFromEmail(email) {
  return slugify(accountName(email)) || 'account';
}

// Pick an unused profile name, appending -2, -3, ... when taken.
function uniqueProfileName(base) {
  let target = base;
  let n = 2;
  while (fs.existsSync(profileDir(target))) target = `${base}-${n++}`;
  return target;
}

function createProfileDir(name) {
  const dir = profileDir(name);
  fs.mkdirSync(path.join(dir, 'projects'), { recursive: true });
  return dir;
}

function runLoginFor(name, loginOpts) {
  const dir = createProfileDir(name);
  const email = loginOpts && loginOpts.email;
  process.stdout.write(
    `${C.g}created ${dir}${C.r}\n` +
      `logging in${email ? ` as ${email}` : ''}... (finish the browser flow, then you are back here)\n\n`
  );
  const code = runClaude(loginArgs(email, loginOpts), profileEnv(name));
  const info = summarizeNewProfile(name);
  if (email && info && info.email && info.email.toLowerCase() !== email.toLowerCase()) {
    process.stdout.write(
      `${C.y}warning: signed in as ${info.email}, not ${email}.${C.r}\n` +
        `${C.d}if that is wrong, run: ccx login ${name} --email ${email}${C.r}\n`
    );
  }
  return code;
}

function addProfile(name, loginOpts) {
  if (name === 'default') {
    process.stderr.write('"default" is reserved for ~/.claude\n');
    return 1;
  }
  const dir = profileDir(name);
  if (fs.existsSync(dir) && fs.readdirSync(dir).length) {
    process.stdout.write(`${C.y}profile "${name}" already exists at ${dir}${C.r}\n`);
    return 1;
  }
  return runLoginFor(name, loginOpts);
}

// After a login, tell the user who they signed in as and offer a readable label.
function summarizeNewProfile(name) {
  const info = describeWithSummary(name, { skipCli: false });
  if (!info.email) {
    process.stdout.write(`${C.y}could not read the account for "${name}" yet${C.r}\n`);
    return null;
  }
  process.stdout.write(
    `\n${C.g}profile "${name}" is now signed in as ${info.email}${info.plan ? ` (${info.plan})` : ''}${C.r}\n`
  );
  const suggested = accountName(info.email);
  const current = info.label || name;
  if (suggested && current !== suggested) {
    process.stdout.write(
      `${C.d}shown as "${current}" in the picker. ` +
        `Run \`ccx label ${name} ${suggested}\` (or press e) to show "${suggested}" instead.${C.r}\n`
    );
  }
  if (info.auth === 'logged-out') {
    process.stdout.write(`${C.y}warning: still not logged in${C.r}\n`);
  }
  return info;
}

// `ccx add` with no name: log in first, then name the profile after the account.
function addAutoProfile(loginOpts) {
  const temp = `new-${Date.now().toString(36).slice(-5)}`;
  const code = runLoginFor(temp, loginOpts);
  if (!fs.existsSync(profileDir(temp))) return code;
  const info = describeWithSummary(temp, { skipCli: false });
  if (!info.email) {
    process.stdout.write(
      `${C.y}logged in but the account is unknown; profile kept as "${temp}". ` +
        `Rename it with \`ccx rename ${temp} <name>\`.${C.r}\n`
    );
    return 0;
  }
  const base = profileNameFromEmail(info.email) || temp;
  const target = uniqueProfileName(base);
  fs.renameSync(profileDir(temp), profileDir(target));
  writeLabel(target, accountName(info.email));
  process.stdout.write(
    `${C.g}named the profile "${target}" (from ${info.email})${C.r}\n` +
      `${C.d}rename with: ccx rename ${target} <name>${C.r}\n`
  );
  return 0;
}

// Mark the current directory trusted in a profile's .claude.json
// (silences "this workspace has not been trusted" and enables settings.json permissions).
function trustDir(name, dir) {
  const cfgPath = name === 'default' ? path.join(HOME, '.claude.json') : path.join(profileDir(name), '.claude.json');
  let o = {};
  try {
    o = JSON.parse(fs.readFileSync(cfgPath, 'utf8'));
  } catch {}
  o.projects = o.projects || {};
  const key = dir.replace(/\\/g, '/').replace(/\/$/, '');
  o.projects[key] = { ...(o.projects[key] || {}), hasTrustDialogAccepted: true };
  fs.writeFileSync(cfgPath, JSON.stringify(o, null, 2));
  process.stdout.write(`${C.g}trusted ${key} in ${name}${C.r}\n`);
  return 0;
}

function loginProfile(name, loginOpts) {
  if (!listProfileNames().includes(name)) {
    process.stderr.write(`unknown profile "${name}"\n`);
    return 1;
  }
  const email = loginOpts && loginOpts.email;
  if (email) process.stdout.write(`${C.d}pre-filling ${email} on the login page${C.r}\n`);
  const code = runClaude(loginArgs(email, loginOpts), profileEnv(name));
  const info = summarizeNewProfile(name);
  if (email && info && info.email && info.email.toLowerCase() !== email.toLowerCase()) {
    process.stdout.write(
      `${C.y}warning: signed in as ${info.email}, not ${email}.${C.r}\n`
    );
  }
  return code;
}

function renameProfile(from, to) {
  if (from === 'default' || to === 'default') {
    process.stderr.write('"default" is reserved for ~/.claude and cannot be renamed\n');
    return 1;
  }
  const src = profileDir(from);
  const dst = profileDir(to);
  if (!fs.existsSync(src)) {
    process.stderr.write(`profile "${from}" does not exist\n`);
    return 1;
  }
  if (fs.existsSync(dst)) {
    process.stderr.write(`profile "${to}" already exists at ${dst}\n`);
    return 1;
  }
  fs.renameSync(src, dst);
  const label = readLabel(from);
  if (label) {
    writeLabel(to, label);
    writeLabel(from, '');
  }
  process.stdout.write(`${C.g}renamed ${from} -> ${to}${C.r}  ${dst}\n`);
  return 0;
}

// Copy an existing profile's auth (and settings) into a new one.
function cloneProfile(src, dst, opts) {
  if (dst === 'default') {
    process.stderr.write('"default" is reserved for ~/.claude\n');
    return 1;
  }
  if (!fs.existsSync(profileDir(src))) {
    process.stderr.write(`source profile "${src}" does not exist\n`);
    return 1;
  }
  const target = profileDir(dst);
  if (fs.existsSync(target) && fs.readdirSync(target).length) {
    process.stderr.write(`target "${dst}" already exists at ${target}\n`);
    return 1;
  }
  fs.mkdirSync(path.join(target, 'projects'), { recursive: true });
  const copied = [];
  const tryCopy = (file) => {
    const from = path.join(profileDir(src), file);
    if (!fs.existsSync(from)) return;
    fs.copyFileSync(from, path.join(target, file));
    copied.push(file);
  };
  if (!opts.noAuth) tryCopy('.credentials.json');
  tryCopy('settings.json');
  tryCopy('settings.local.json');
  // minimal .claude.json so onboarding is skipped
  const srcJson = [path.join(profileDir(src), '.claude.json'), path.join(HOME, '.claude.json')].find(
    (p) => fs.existsSync(p)
  );
  if (srcJson) {
    try {
      const o = JSON.parse(fs.readFileSync(srcJson, 'utf8'));
      const projects = {};
      for (const [dir, cfg] of Object.entries(o.projects || {})) {
        if (!cfg || typeof cfg !== 'object') continue;
        const entry = {};
        for (const k of ['hasTrustDialogAccepted', 'allowedTools', 'hasCompletedProjectOnboarding']) {
          if (cfg[k] !== undefined) entry[k] = cfg[k];
        }
        if (Object.keys(entry).length) projects[dir] = entry;
      }
      const minimal = {
        userID: o.userID,
        oauthAccount: opts.noAuth ? undefined : o.oauthAccount,
        hasCompletedOnboarding: true,
        lastOnboardingVersion: o.lastOnboardingVersion,
        numStartups: 1,
        installMethod: o.installMethod,
        projects,
      };
      fs.writeFileSync(path.join(target, '.claude.json'), JSON.stringify(minimal, null, 2));
      copied.push('.claude.json');
    } catch {}
  }
  process.stdout.write(`${C.g}cloned ${src} -> ${dst}${C.r} (${copied.join(', ') || 'empty'})\n`);
  if (opts.noAuth) {
    process.stdout.write(`run \`ccx login ${dst}\` to sign in, or \`ccx ${dst}\` and use /login\n`);
  } else {
    process.stdout.write(
      `same account as ${src}. Use \`ccx login ${dst}\` (or /login inside) to switch account.\n`
    );
  }
  return 0;
}

// ------------------------------------------------------------------------- TUI

function readKey() {
  return new Promise((resolve) => {
    const onData = (buf) => {
      cleanup();
      const s = buf.toString('utf8');
      if (s === '\u001b[A' || s === '\u001bOA') return resolve('up');
      if (s === '\u001b[B' || s === '\u001bOB') return resolve('down');
      if (s === '\r' || s === '\n' || s === '\r\n') return resolve('enter');
      if (s === '\u0003' || s === '\u001b') return resolve('quit');
      if (s === '\u000e') return resolve('down');
      if (s === '\u0010') return resolve('up');
      if (/^[0-9]$/.test(s)) return resolve('num:' + s);
      resolve(s.toLowerCase());
    };
    const cleanup = () => {
      process.stdin.removeListener('data', onData);
      process.stdin.setRawMode(false);
      process.stdin.pause();
    };
    process.stdin.resume();
    process.stdin.setRawMode(true);
    process.stdin.on('data', onData);
  });
}

function promptLine(question) {
  return new Promise((resolve) => {
    process.stdout.write(question);
    const rl = readline.createInterface({ input: process.stdin, output: process.stdout });
    rl.question('', (answer) => {
      rl.close();
      resolve(answer.trim());
    });
  });
}

// Fallback for non-TTY stdin (CI, piped stdin): plain numbered list.
async function pickPlain(profiles) {
  process.stdout.write('#  profile        account\n');
  profiles.forEach((p, i) => {
    process.stdout.write(
      `${i + 1}  ${p.name.padEnd(14)} ${p.email || p.auth || '-'}${p.plan ? ` (${p.plan})` : ''}\n`
    );
  });
  const answer = await promptLine(`pick 1-${profiles.length} [1]: `);
  const idx = (answer === '' ? 1 : Number(answer)) - 1;
  if (!(idx >= 0 && idx < profiles.length)) {
    process.stderr.write('invalid choice\n');
    return null;
  }
  return profiles[idx];
}

function clearScreen() {
  process.stdout.write('\x1b[2J\x1b[H');
}

async function pick(initialProfiles) {
  const interactive = process.stdin.isTTY && typeof process.stdin.setRawMode === 'function';
  if (!interactive) return pickPlain(initialProfiles);
  let profiles = initialProfiles;
  let cursor = 0;
  let bestAge = -1;
  profiles.forEach((p, i) => {
    if (p.lastUsed > bestAge) {
      bestAge = p.lastUsed;
      cursor = i;
    }
  });
  let status = '';

  while (true) {
    clearScreen();
    const out = [];
    out.push(`${C.c}ccx${C.r} ${C.d}— choose a Claude Code account${C.r}`);
    out.push('');
    profiles.forEach((p, i) => {
      const sel = i === cursor ? `${C.g}❯${C.r}` : ' ';
      const num = `${C.d}${i + 1}${C.r}`;
      out.push(` ${sel} ${num}  ${profileLine(p)}`);
    });
    out.push('');
    const yolo =
      process.env.CCX_SAFE === '1' ? `${C.y}skip-permissions OFF${C.r}` : `${C.g}YOLO${C.r}`;
    out.push(
      `${C.d}↑↓/jk move · enter launch · l login · a add · e label · r refresh · q quit${C.r}  ${yolo}`
    );
    if (status) out.push(`${C.y}${status}${C.r}`);
    process.stdout.write(out.join('\n') + '\n');

    const key = await readKey();
    status = '';
    if (key === 'quit') {
      clearScreen();
      process.stdout.write(SHOW_CURSOR);
      return null;
    }
    if (key === 'up') cursor = (cursor - 1 + profiles.length) % profiles.length;
    else if (key === 'down') cursor = (cursor + 1) % profiles.length;
    else if (key === 'j') cursor = (cursor + 1) % profiles.length;
    else if (key === 'k') cursor = (cursor - 1 + profiles.length) % profiles.length;
    else if (key.startsWith('num:')) {
      const idx = Number(key.slice(4)) - 1;
      if (idx >= 0 && idx < profiles.length) {
        const chosen = profiles[idx];
        clearScreen();
        process.stdout.write(SHOW_CURSOR);
        return chosen;
      }
      status = `no profile ${key.slice(4)}`;
    } else if (key === 'enter') {
      clearScreen();
      process.stdout.write(SHOW_CURSOR);
      return profiles[cursor];
    } else if (key === 'r') {
      process.stdout.write(SHOW_CURSOR);
      profiles = listProfileNames().map(describe);
      cursor = Math.min(cursor, profiles.length - 1);
    } else if (key === 'l') {
      process.stdout.write(SHOW_CURSOR);
      clearScreen();
      const target = profiles[cursor];
      const answer = await promptLine(
        `email to pre-fill for ${target.name}${target.email ? ` [${target.email}]` : ''} (blank = any): `
      );
      const email = EMAIL_RE.test(answer) ? answer : target.email || null;
      const code = loginProfile(target.name, { email });
      profiles = listProfileNames().map(describe);
      status = code === 0 ? `logged in ${profiles[cursor].name}` : `login exited ${code}`;
    } else if (key === 'e') {
      process.stdout.write(SHOW_CURSOR);
      clearScreen();
      const target = profiles[cursor];
      const hint = target.email ? accountName(target.email) : '';
      const label = await promptLine(`label for ${target.name}${hint ? ` [${hint}]` : ''}: `);
      if (label !== undefined) {
        writeLabel(target.name, label);
        profiles = listProfileNames().map(describe);
        status = label ? `label set to ${label}` : 'label cleared';
      }
    } else if (key === 'a') {
      process.stdout.write(SHOW_CURSOR);
      clearScreen();
      const answer = await promptLine(
        'new profile: name, or email to sign in as <you@example.com>: '
      );
      if (!answer || answer === '\u0003') {
        status = 'cancelled';
      } else if (EMAIL_RE.test(answer)) {
        const name = uniqueProfileName(profileNameFromEmail(answer));
        addProfile(name, { email: answer });
        profiles = listProfileNames().map(describe);
        const idx = profiles.findIndex((p) => p.name === name);
        if (idx >= 0) cursor = idx;
        status = `added ${name} for ${answer}`;
      } else {
        addProfile(answer);
        profiles = listProfileNames().map(describe);
        const idx = profiles.findIndex((p) => p.name === answer);
        if (idx >= 0) cursor = idx;
        status = `profile ${answer} added`;
      }
    }
  }
}

// ------------------------------------------------------------------------ main

// Split --email/--console/--sso out of argv; leftovers stay for the profile name.
function parseLoginOpts(args) {
  const opts = { email: null, console: false, sso: false };
  for (let i = 0; i < args.length; i++) {
    const a = args[i];
    if (a === '--email') opts.email = args[++i] || null;
    else if (a.startsWith('--email=')) opts.email = a.slice(8);
    else if (a === '--console' || a === '--api') opts.console = true;
    else if (a === '--sso') opts.sso = true;
  }
  if (opts.email && !EMAIL_RE.test(opts.email)) opts.email = null;
  return opts;
}

async function main() {
  let argv = process.argv.slice(2);
  // global flags usable anywhere on the command line
  const noYolo = argv.some((a) => a === '--safe' || a === '--no-yolo');
  if (noYolo) {
    process.env.CCX_SAFE = '1';
    argv = argv.filter((a) => a !== '--safe' && a !== '--no-yolo');
  }
  const first = argv[0];

  if (first === 'list' || first === 'ls') {
    const names = listProfileNames();
    const rows = names.map((n) => describeWithSummary(n, { skipCli: false }));
    const showLabel = rows.some((p) => p.label);
    const head = ['profile', ...(showLabel ? ['label'] : []), 'account', 'plan', 'last used'];
    const widths = [16, ...(showLabel ? [18] : []), 34, 8];
    const pad = (s, w) => String(s == null ? '' : s).slice(0, w).padEnd(w);
    process.stdout.write(
      head.map((h, i) => pad(h, widths[i])).join(' ') + ' path\n'
    );
    for (const p of rows) {
      const cols = [p.name, ...(showLabel ? [p.label || ''] : []), p.email || p.auth || '-', p.plan || ''];
      process.stdout.write(
        cols.map((c, i) => pad(c, widths[i])).join(' ') +
          ` ${humanAge(p.lastUsed).padEnd(9)} ${C.d}${p.dir}${C.r}\n`
      );
    }
    return 0;
  }
  if (first === 'label') {
    if (!argv[1]) {
      const rows = listProfileNames().map((n) => describeWithSummary(n, { skipCli: false }));
      for (const p of rows) {
        process.stdout.write(
          `${p.name.padEnd(16)} ${(p.label || '').padEnd(18)} ${p.email || p.auth || '-'}\n`
        );
      }
      return 0;
    }
    const name = argv[1];
    if (!listProfileNames().includes(name)) {
      process.stderr.write(`unknown profile "${name}"\n`);
      return 1;
    }
    const label = argv.slice(2).join(' ').trim();
    writeLabel(name, label);
    process.stdout.write(
      label
        ? `${C.g}${name} now shows as "${label}" in the picker${C.r}\n`
        : `${C.g}label cleared for ${name}${C.r}\n`
    );
    return 0;
  }
  if (first === 'account' || first === 'whoami') {
    const name = argv[1] || 'default';
    const info = describeWithSummary(name, { skipCli: false });
    process.stdout.write(
      [
        `profile   ${info.name}`,
        `label     ${info.label || '(none)'}`,
        `email     ${info.email || '-'}`,
        `account   ${accountName(info.email) || '-'}`,
        `plan      ${info.plan || '-'}`,
        `auth      ${info.auth || '-'}`,
        `dir       ${info.dir}`,
        `last used ${humanAge(info.lastUsed)}`,
      ].join('\n') + '\n'
    );
    return 0;
  }
  if (first === 'add') {
    const opts = parseLoginOpts(argv.slice(1));
    const target = argv.slice(1).find((a) => !a.startsWith('-'));
    if (target && EMAIL_RE.test(target)) {
      // ccx add you@example.com → profile named after the email
      const name = uniqueProfileName(profileNameFromEmail(target));
      return addProfile(name, { ...opts, email: target });
    }
    if (target) return addProfile(target, { ...opts, email: opts.email });
    return addAutoProfile(opts.email ? { ...opts } : opts);
  }
  if (first === 'login' && argv[1]) {
    const opts = parseLoginOpts(argv.slice(2));
    if (EMAIL_RE.test(argv[1])) {
      const email = argv[1];
      const found = listProfileNames()
        .map((n) => describeWithSummary(n, { skipCli: true }))
        .find((p) => p.email && p.email.toLowerCase() === email.toLowerCase());
      if (found) {
        process.stdout.write(`${C.d}"${email}" already has profile "${found.name}"${C.r}\n`);
        return loginProfile(found.name, { ...opts, email });
      }
      const name = uniqueProfileName(profileNameFromEmail(email));
      process.stdout.write(`${C.d}no profile matches that email; creating "${name}"${C.r}\n`);
      return addProfile(name, { ...opts, email });
    }
    return loginProfile(argv[1], opts);
  }
  if ((first === 'rename' || first === 'mv') && argv[1] && argv[2]) {
    return renameProfile(argv[1], argv[2]);
  }
  if (first === 'trust') {
    const name = argv[1] || 'default';
    const dir = argv[2] ? path.resolve(argv[2]) : process.cwd();
    return trustDir(name, dir);
  }
  if ((first === 'clone' || first === 'cp') && argv[1] && argv[2]) {
    const noAuth = argv.includes('--no-auth');
    return cloneProfile(argv[1], argv[2], { noAuth });
  }
  if (first === 'help' || first === '-h' || first === '--help') {
    const src = fs.readFileSync(__filename, 'utf8');
    process.stdout.write(src.slice(src.indexOf('/**') + 3, src.indexOf('*/')) + '\n');
    return 0;
  }
  if (first === 'help') return 0;

  // explicit profile: ccx <name> [claude args...]
  if (first && !first.startsWith('-')) {
    const names = listProfileNames();
    if (names.includes(first)) {
      return runClaude(withYolo(argv.slice(1)), profileEnv(first));
    }
    process.stderr.write(
      `unknown profile "${first}". known: ${names.join(', ')}\n` +
        `create it with: ccx add <name>   (or run \`ccx\` for the picker)\n`
    );
    return 1;
  }

  // TUI path
  const profiles = listProfileNames().map(describe);
  process.stdout.write(SHOW_CURSOR);
  const chosen = await pick(profiles);
  if (!chosen) return 130;
  if (!chosen.exists) {
    const code = addProfile(chosen.name);
    if (code !== 0) return code;
  }
  const passthrough = first === '--' ? argv.slice(1) : argv;
  process.stdout.write(`${C.d}launching claude as ${chosen.name}${C.r}\n`);
  return runClaude(withYolo(passthrough), profileEnv(chosen.name));
}

main().then(
  (code) => process.exit(code || 0),
  (err) => {
    process.stdout.write(SHOW_CURSOR);
    process.stderr.write(String(err && err.stack ? err.stack : err) + '\n');
    process.exit(1);
  }
);
