import { readdir, readFile } from 'node:fs/promises';
import { extname, join, relative, resolve } from 'node:path';
import { gzipSync } from 'node:zlib';

const root = resolve('dist');
const limits = {
  '.css': 40 * 1024,
  '.html': 12 * 1024,
  '.js': 540 * 1024,
  '.wasm': 350 * 1024,
} as const;

const totals = new Map<string, number>();
const files: string[] = [];

async function walk(directory: string): Promise<void> {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      await walk(path);
    } else {
      files.push(path);
    }
  }
}

await walk(root);
for (const file of files) {
  if (file.endsWith('.map')) continue;
  const extension = extname(file);
  if (!(extension in limits)) continue;
  const bytes = gzipSync(await readFile(file)).byteLength;
  totals.set(extension, (totals.get(extension) ?? 0) + bytes);
}

const failures: string[] = [];
for (const [extension, limit] of Object.entries(limits)) {
  const total = totals.get(extension) ?? 0;
  const formatted = `${(total / 1024).toFixed(1)} KiB / ${(limit / 1024).toFixed(0)} KiB`;
  if (total > limit) failures.push(`${extension}: ${formatted}`);
  else process.stdout.write(`${extension}: ${formatted}\n`);
}

for (const file of files) {
  if (!/\.(?:css|html|js)$/.test(file) || file.endsWith('.map')) continue;
  const contents = await readFile(file, 'utf8');
  const forbiddenBuildStrings = [
    ['use.typekit.net', 'external Typekit reference'],
    ['The style macro must be imported', 'untransformed Spectrum style macro'],
    ['fileURLToPath', 'Node-only Spectrum macro runtime'],
  ] as const;
  for (const [pattern, reason] of forbiddenBuildStrings) {
    if (contents.includes(pattern)) {
      failures.push(`${relative(root, file)}: ${reason}`);
    }
  }
}

// The initial Spectrum shell paints before the independent editor and engine
// chunks. Neither entry may move authoring code or WASM onto that critical
// preload path.
for (const page of ['index.html', 'gallery.html']) {
  const html = await readFile(join(root, page), 'utf8');
  const deferredChunks = [
    'vendor-codemirror',
    'adapter-engine',
    'wasm-loader',
    '.wasm',
  ];
  if (page === 'gallery.html') deferredChunks.push('vendor-lz-string');
  for (const deferredChunk of deferredChunks) {
    const escapedChunk = deferredChunk.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    const preload = new RegExp(
      `<link[^>]+rel=["'](?:module)?preload["'][^>]+${escapedChunk}`,
      'i',
    );
    if (preload.test(html)) {
      failures.push(
        `${page}: ${deferredChunk} must remain off the preload path`,
      );
    }
  }
}

if (failures.length > 0) {
  throw new Error(
    `Production bundle budget exceeded in ${relative(process.cwd(), root)}:\n${failures.join('\n')}`,
  );
}
