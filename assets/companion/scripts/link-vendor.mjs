// Links vendored packages into node_modules so their bare imports resolve offline.
// Each vendor/<dir> is named <scope>-<pkg>-<version>; the package name is read from its package.json.
import { mkdirSync, readdirSync, readFileSync, rmSync, symlinkSync, existsSync } from 'node:fs';
import { join, relative, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const vendor = join(root, 'vendor');
const nm = join(root, 'node_modules');
rmSync(nm, { recursive: true, force: true });
for (const dir of readdirSync(vendor, { withFileTypes: true })) {
  if (!dir.isDirectory()) continue;
  const pkgJson = join(vendor, dir.name, 'package.json');
  if (!existsSync(pkgJson)) continue;
  const { name } = JSON.parse(readFileSync(pkgJson, 'utf8'));
  const link = join(nm, name);
  mkdirSync(dirname(link), { recursive: true });
  symlinkSync(relative(dirname(link), join(vendor, dir.name)), link, 'dir');
}
