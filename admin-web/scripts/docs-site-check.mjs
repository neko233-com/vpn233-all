import { createServer } from 'node:http';
import { readFile, readdir, realpath, stat } from 'node:fs/promises';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { chromium } from '@playwright/test';

const types = { '.html': 'text/html; charset=utf-8', '.css': 'text/css', '.js': 'text/javascript', '.svg': 'image/svg+xml', '.png': 'image/png', '.ico': 'image/x-icon', '.json': 'application/json' };

export async function validateSite(directory, baseurl = '') {
  const root = await realpath(directory);
  if (baseurl && (!baseurl.startsWith('/') || baseurl.includes('..') || baseurl.includes('?') || baseurl.includes('#'))) throw new Error('docs-site-invalid-baseurl');
  const base = baseurl.replace(/\/$/, '');
  const errors = new Set();
  const counts = { pages: 0, links: 0, external_requests: 0, browser_errors: 0, http_errors: 0 };
  async function targetFile(pathname) {
    const decoded = decodeURIComponent(pathname);
    if (base && decoded !== base && !decoded.startsWith(base + '/')) throw new Error('docs-site-path-outside-base');
    const relative = decoded.slice(base.length).replace(/^\/+/, '');
    let candidate = path.resolve(root, relative);
    if (candidate !== root && !candidate.startsWith(root + path.sep)) throw new Error('docs-site-path-outside-root');
    if ((await stat(candidate)).isDirectory()) candidate = path.join(candidate, 'index.html');
    const resolved = await realpath(candidate);
    if (!resolved.startsWith(root + path.sep) || !(await stat(resolved)).isFile()) throw new Error('docs-site-invalid-target');
    return resolved;
  }
  async function htmlFiles(folder) {
    const files = [];
    for (const entry of await readdir(folder, { withFileTypes: true })) {
      const file = path.join(folder, entry.name);
      if (entry.isDirectory()) files.push(...await htmlFiles(file));
      else if (entry.isFile() && entry.name.endsWith('.html')) files.push(file);
      else if (entry.isSymbolicLink()) throw new Error('docs-site-symlink-not-allowed');
    }
    return files.sort();
  }
  const files = await htmlFiles(root);
  if (!files.length) throw new Error('docs-site-no-html-pages');
  const server = createServer(async (request, response) => {
    try {
      const file = await targetFile(new URL(request.url, 'http://127.0.0.1').pathname);
      response.writeHead(200, { 'Content-Type': types[path.extname(file)] || 'application/octet-stream', 'Cache-Control': 'no-store' });
      response.end(await readFile(file));
    } catch {
      response.writeHead(404, { 'Content-Type': 'text/plain' });
      response.end('Not found');
    }
  });
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
  const origin = `http://127.0.0.1:${server.address().port}`;
  let browser;
  try {
    browser = await chromium.launch({ headless: true, args: ['--disable-background-networking', '--disable-component-update', '--disable-sync', '--no-first-run'] });
    const context = await browser.newContext({ serviceWorkers: 'block' });
    await context.route('**/*', async (route) => {
      if (new URL(route.request().url()).origin !== origin) {
        counts.external_requests++;
        errors.add('external-request');
        await route.abort();
      } else await route.continue();
    });
    const page = await context.newPage();
    page.on('pageerror', () => { counts.browser_errors++; errors.add('browser-error'); });
    page.on('console', (event) => { if (event.type() === 'error') { counts.browser_errors++; errors.add('browser-error'); } });
    page.on('response', (response) => { if (response.status() >= 400) { counts.http_errors++; errors.add('http-error'); } });
    const anchors = new Map();
    for (const file of files) {
      const relative = path.relative(root, file).split(path.sep).map(encodeURIComponent).join('/');
      const response = await page.goto(origin + base + '/' + relative, { waitUntil: 'networkidle', timeout: 15000 });
      if (!response?.ok()) errors.add('page-not-successful');
      counts.pages++;
      const links = await page.locator('a[href]').evaluateAll((items) => items.map((item) => item.getAttribute('href')));
      for (const href of links) {
        if (!href || /^(mailto:|tel:)/i.test(href)) continue;
        const destination = new URL(href, page.url());
        if (['http:', 'https:'].includes(destination.protocol) && destination.origin !== origin) continue;
        if (destination.protocol !== 'http:' || destination.origin !== origin) { errors.add('invalid-link-protocol'); continue; }
        counts.links++;
        let target;
        try { target = await targetFile(destination.pathname); } catch { errors.add('broken-local-link'); continue; }
        if (destination.hash && ['.html', '.svg'].includes(path.extname(target))) {
          let identifiers = anchors.get(target);
          if (!identifiers) {
            const content = await readFile(target, 'utf8');
            identifiers = new Set(await page.evaluate((source) => [...new DOMParser().parseFromString(source, 'text/html').querySelectorAll('[id],a[name]')].flatMap((item) => [item.id, item.getAttribute('name')].filter(Boolean)), content));
            anchors.set(target, identifiers);
          }
          try { if (!identifiers.has(decodeURIComponent(destination.hash.slice(1)))) errors.add('broken-anchor'); } catch { errors.add('invalid-anchor-encoding'); }
        }
      }
    }
    await context.close();
    if (errors.size) throw new Error('docs-site-validation-failed:' + [...errors].sort().join(','));
    return counts;
  } finally {
    await browser?.close();
    await new Promise((resolve) => server.close(resolve));
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  const options = new Map();
  for (let i = 2; i < process.argv.length; i += 2) options.set(process.argv[i], process.argv[i + 1]);
  try {
    if (!options.get('--root') || [...options.keys()].some((key) => !['--root', '--baseurl'].includes(key))) throw new Error('docs-site-invalid-arguments');
    console.log('Local rendered documentation: ' + JSON.stringify(await validateSite(options.get('--root'), options.get('--baseurl') || '')));
  } catch {
    // The failing rule is useful; raw browser errors/URLs/local paths are private.
    console.error('Local rendered documentation: FAILED (details redacted)');
    process.exitCode = 1;
  }
}
