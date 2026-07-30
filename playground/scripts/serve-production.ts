import { extname, resolve, sep } from 'node:path';

const root = resolve('dist');
const base = '/aozora/playground/';
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

function assetPath(pathname: string): string | null {
  if (pathname === base.slice(0, -1)) return '';
  if (!pathname.startsWith(base)) return null;
  const relative =
    decodeURIComponent(pathname.slice(base.length)) || 'index.html';
  const path = resolve(root, relative);
  return path === root || path.startsWith(`${root}${sep}`) ? path : null;
}

const server = Bun.serve({
  hostname: '127.0.0.1',
  port: 5173,
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
    const acceptsGzip = request.headers
      .get('accept-encoding')
      ?.split(',')
      .some((value) => value.trim().startsWith('gzip'));
    if (acceptsGzip && compressible.has(extname(path))) {
      let compressed = gzipCache.get(path);
      if (!compressed) {
        compressed = Bun.gzipSync(await file.bytes());
        gzipCache.set(path, compressed);
      }
      headers.set('Content-Encoding', 'gzip');
      headers.set('Vary', 'Accept-Encoding');
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
