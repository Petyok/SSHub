#!/usr/bin/env node
// npm entry point for SSHub.
//
// The real binary lives in a per-platform package (`sshub-linux-x64` and
// friends) that npm installs as an optional dependency and skips everywhere its
// `os`/`cpu` do not match. So this shim never downloads anything: it resolves
// the one package that did install and hands the terminal over to it.
'use strict';

const { spawnSync } = require('node:child_process');
const fs = require('node:fs');

/** Platform key -> package carrying that platform's binary. */
const PACKAGES = {
  'linux-x64': 'sshub-linux-x64',
  'darwin-arm64': 'sshub-darwin-arm64',
  'darwin-x64': 'sshub-darwin-x64',
};

function die(message) {
  process.stderr.write(`sshub: ${message}\n`);
  process.exit(1);
}

function main() {
  const platform = `${process.platform}-${process.arch}`;
  const pkg = PACKAGES[platform];
  if (!pkg) {
    die(
      `no prebuilt binary for ${platform}. ` +
        `Prebuilt platforms: ${Object.keys(PACKAGES).join(', ')}. ` +
        'Build from source instead: cargo install sshub',
    );
  }

  let bin;
  try {
    bin = require.resolve(`${pkg}/bin/sshub`);
  } catch {
    // The optional dependency is missing, which usually means the install ran
    // with --no-optional or was interrupted.
    die(`${pkg} is not installed. Reinstall with: npm install -g sshub`);
  }

  // npm preserves the executable bit, but a zip-based store or an over-eager
  // umask can drop it. Cheaper to fix than to explain.
  try {
    fs.accessSync(bin, fs.constants.X_OK);
  } catch {
    try {
      fs.chmodSync(bin, 0o755);
    } catch {
      die(`${bin} is not executable`);
    }
  }

  // `stdio: 'inherit'` hands over the real tty, which the TUI needs, and puts
  // the child in the same process group so Ctrl+C reaches it from the terminal
  // rather than through us.
  const result = spawnSync(bin, process.argv.slice(2), { stdio: 'inherit' });
  if (result.error) {
    die(`could not start ${bin}: ${result.error.message}`);
  }
  if (result.signal) {
    // Die the same way the binary did, so a shell sees the real cause.
    process.kill(process.pid, result.signal);
    return;
  }
  process.exit(result.status ?? 1);
}

main();
