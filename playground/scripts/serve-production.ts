import { extname, resolve, sep } from 'node:path';

const root = resolve('dist');
const base = '/aozora/playground/';
const port = Number(Bun.env.PLAYGROUND_PORT ?? 5173);
const compressible = new Set([
  '.css',
  '.html',
  '.js',
  '.json',
  '.map',
  '.svg',
  '.wasm',
]);
const gzipCache = new Map<string, Uint8Array>();

function acceptsGzip(header: string | null): boolean {
  if (header === null) return false;
  const encodings = new Map<string, number>();
  for (const value of header.split(',')) {
    const [name, ...parameters] = value.trim().split(';');
    if (!name) continue;
    const qualityParameter = parameters.find((parameter) =>
      parameter.trim().toLowerCase().startsWith('q='),
    );
    const quality =
      qualityParameter === undefined
        ? 1
        : Number(qualityParameter.trim().slice(2));
    encodings.set(name.toLowerCase(), quality);
  }
  return (encodings.get('gzip') ?? encodings.get('*') ?? 0) > 0;
}

function assetPath(pathname: string): string | null {
  if (pathname === base.slice(0, -1)) return '';
  if (!pathname.startsWith(base)) return null;
  let relative: string;
  try {
    relative = decodeURIComponent(pathname.slice(base.length)) || 'index.html';
  } catch {
    return null;
  }
  const path = resolve(root, relative);
  return path === root || path.startsWith(`${root}${sep}`) ? path : null;
}

const server = Bun.serve({
  hostname: '127.0.0.1',
  port,
  async fetch(request) {
    const url = new URL(request.url);
    const path = assetPath(url.pathname);
    if (path === '') {
      return Response.redirect(`${url.origin}${base}`, 308);
    }
    if (path === null) return new Response('Not found', { status: 404 });

    const file = Bun.file(path);
    if (!(await file.exists()))
      return new Response('Not found', { status: 404 });

    const headers = new Headers({
      'Cache-Control': path.includes(`${sep}assets${sep}`)
        ? 'public, max-age=31536000, immutable'
        : 'no-cache',
      'Content-Type': file.type || 'application/octet-stream',
    });
    const isCompressible = compressible.has(extname(path));
    if (isCompressible) headers.set('Vary', 'Accept-Encoding');
    if (isCompressible && acceptsGzip(request.headers.get('accept-encoding'))) {
      let compressed = gzipCache.get(path);
      if (!compressed) {
        compressed = Bun.gzipSync(await file.bytes());
        gzipCache.set(path, compressed);
      }
      headers.set('Content-Encoding', 'gzip');
      const body =
        request.method === 'HEAD' ? null : new Uint8Array(compressed);
      return new Response(body, {
        headers,
      });
    }
    return new Response(request.method === 'HEAD' ? null : file, { headers });
  },
});

process.stdout.write(`Production server: ${server.url}${base.slice(1)}\n`);
