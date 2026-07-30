import { readdir, readFile } from 'node:fs/promises';
import { extname, join, relative } from 'node:path';

const roots = ['src', '../playground-ui/src'];
const manifests = ['package.json', 'bun.lock', '../playground-ui/package.json'];
const extensions = new Set(['.ts', '.tsx', '.css']);
const forbidden = [
  ['solid', '-js'].join(''),
  ['@solidjs', '/testing-library'].join(''),
  ['vite-plugin', '-solid'].join(''),
  ['UNSAFE', '_'].join(''),
  ['IR', ' JSON'].join(''),
  ['Nodes', ' JSON'].join(''),
  ['HTML', ' source'].join(''),
  ['Perf', 'Badge'].join(''),
  ['Code', 'View'].join(''),
];

async function filesUnder(directory: string): Promise<string[]> {
  const entries = await readdir(directory, { withFileTypes: true });
  const files: string[] = [];
  for (const entry of entries) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) files.push(...(await filesUnder(path)));
    else if (extensions.has(extname(entry.name))) files.push(path);
  }
  return files;
}

const files = [
  ...manifests,
  ...(await Promise.all(roots.map(filesUnder))).flat(),
];
const violations: string[] = [];
for (const path of files) {
  const source = await readFile(path, 'utf8');
  for (const pattern of forbidden) {
    if (source.includes(pattern)) {
      violations.push(
        `${relative('.', path)} contains ${JSON.stringify(pattern)}`,
      );
    }
  }
}

if (violations.length > 0) {
  throw new Error(
    `Legacy playground surfaces remain:\n${violations.join('\n')}`,
  );
}
