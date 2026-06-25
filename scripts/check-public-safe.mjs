#!/usr/bin/env node
/**
 * Public-safety lint — blocks internal machine paths, vault slugs, internal
 * names, and secrets from a repo intended for public release.
 *
 * Scans ALL tracked text files (not just docs) so leaks in source/config/CI
 * are caught too. Skips binaries and lockfiles. The scanner and the denylist
 * files exclude themselves (they legitimately contain the patterns).
 *
 * The pattern set is NOT hard-coded here — it loads from a JSON denylist so the
 * gate stays public-safe without shipping real internal slugs:
 *
 *   1. `.public-safe-denylist.local.json`   (gitignored — your real patterns)
 *   2. `.public-safe-denylist.example.json` (committed — generic placeholders)
 *
 * `.local.json` wins when present; otherwise the committed example is used so
 * CI still runs a functional (if generic) gate. Copy the example to the local
 * file and fill in your real internal slugs/secrets there.
 *
 * Each rule: { "name": string, "re": string, "flags"?: string, "hint"?: string }
 *
 * Per-line escape: put `public-safe-ok: <reason>` in a comment on the line.
 *
 * Usage:
 *   node scripts/check-public-safe.mjs [files...]   # explicit files
 *   node scripts/check-public-safe.mjs              # all tracked files
 */
import { execSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = (() => {
  try {
    return execSync('git rev-parse --show-toplevel', { encoding: 'utf8' }).trim();
  } catch {
    return resolve(dirname(fileURLToPath(import.meta.url)), '..');
  }
})();

const SELF = 'check-public-safe.mjs';
const DENYLIST_LOCAL = '.public-safe-denylist.local.json';
const DENYLIST_EXAMPLE = '.public-safe-denylist.example.json';

function loadDenylist() {
  const localPath = resolve(ROOT, DENYLIST_LOCAL);
  const examplePath = resolve(ROOT, DENYLIST_EXAMPLE);
  const path = existsSync(localPath) ? localPath : examplePath;
  if (!existsSync(path)) {
    console.error(
      `✗ check-public-safe: no denylist found (looked for ${DENYLIST_LOCAL} then ${DENYLIST_EXAMPLE}).`
    );
    process.exit(1);
  }
  let raw;
  try {
    raw = JSON.parse(readFileSync(path, 'utf8'));
  } catch (e) {
    console.error(`✗ check-public-safe: failed to parse ${path}: ${e.message}`);
    process.exit(1);
  }
  const compile = (arr) =>
    (arr ?? []).map((r) => ({ name: r.name, re: new RegExp(r.re, r.flags ?? ''), hint: r.hint }));
  return {
    block: compile(raw.block),
    warn: compile(raw.warn),
    source: path.replace(`${ROOT}/`, ''),
  };
}

const { block: BLOCK, warn: WARN, source: DENYLIST_SOURCE } = loadDenylist();

const SKIP_EXT = new Set([
  'lock', 'png', 'jpg', 'jpeg', 'gif', 'ico', 'webp', 'bmp',
  'onnx', 'wav', 'mp3', 'ogg', 'flac', 'bin', 'pt', 'safetensors',
  'woff', 'woff2', 'ttf', 'otf', 'eot', 'pdf', 'zip', 'gz', 'tar', 'xz',
  'mp4', 'mov', 'webm', 'wasm',
]);

function trackedFiles() {
  try {
    return execSync('git ls-files', { cwd: ROOT, encoding: 'utf8' }).split('\n').filter(Boolean);
  } catch {
    return [];
  }
}

function isScannable(rel) {
  if (rel.endsWith(SELF)) return false; // never scan the scanner
  // never scan the denylist files — they legitimately hold the patterns
  if (rel.endsWith(DENYLIST_LOCAL) || rel.endsWith(DENYLIST_EXAMPLE)) return false;
  const base = rel.split('/').pop() ?? rel;
  if (base === 'package-lock.json' || base === 'pnpm-lock.yaml' || base === 'yarn.lock' || base === 'Cargo.lock' || base === 'bun.lockb') return false;
  const ext = base.includes('.') ? base.split('.').pop().toLowerCase() : '';
  if (SKIP_EXT.has(ext)) return false;
  return true;
}

const argFiles = process.argv.slice(2).filter((a) => !a.startsWith('-'));
const fileList = (argFiles.length ? argFiles : trackedFiles())
  .map((f) => (f.startsWith('/') ? f.replace(`${ROOT}/`, '') : f))
  .filter(isScannable)
  .filter((f) => existsSync(resolve(ROOT, f)));

if (fileList.length === 0) {
  console.log('✓ check-public-safe: nothing to scan');
  process.exit(0);
}

const blocks = [];
const warns = [];

for (const rel of fileList) {
  let text;
  try {
    text = readFileSync(resolve(ROOT, rel), 'utf8');
  } catch {
    continue;
  }
  if (text.includes('\0')) continue; // binary
  const lines = text.split('\n');
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (line.includes('public-safe-ok:')) continue;
    for (const rule of BLOCK) {
      const m = line.match(rule.re);
      if (m) blocks.push({ rel, lineNo: i + 1, rule: rule.name, snippet: m[0] });
    }
    for (const rule of WARN) {
      const m = line.match(rule.re);
      if (m) warns.push({ rel, lineNo: i + 1, rule: rule.name, snippet: m[0] });
    }
  }
}

if (warns.length) {
  console.warn(`! check-public-safe: ${warns.length} soft warning(s):`);
  for (const w of warns) console.warn(`  ${w.rel}:${w.lineNo} [${w.rule}] ${w.snippet}`);
}

if (blocks.length === 0) {
  console.log(`✓ check-public-safe: ${fileList.length} file(s) clean (denylist: ${DENYLIST_SOURCE})`);
  process.exit(0);
}

console.error(`✗ check-public-safe: ${blocks.length} blocking violation(s) (denylist: ${DENYLIST_SOURCE}):`);
for (const b of blocks) console.error(`  ${b.rel}:${b.lineNo} [${b.rule}] ${b.snippet}`);
console.error('\nFix: scrub internal paths / names / vault slugs / secrets before publishing.');
console.error('Escape (per-line, last resort): add `public-safe-ok: <reason>` in a comment on the line.');
process.exit(1);
