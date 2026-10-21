import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { validateSite } from './docs-site-check.mjs';

async function fixture(html, callback) {
  const root = await mkdtemp(path.join(tmpdir(), 'vpn233-docs-site-test-'));
  try {
    await writeFile(path.join(root, 'index.html'), '<!doctype html><title>fixture</title>' + html);
    await writeFile(path.join(root, 'second.html'), '<!doctype html><title>second</title><h1 id="anchor">fixture</h1>');
    return await callback(root);
  } finally {
    if (path.dirname(path.resolve(root)) !== path.resolve(tmpdir()) || !path.basename(root).startsWith('vpn233-docs-site-test-')) throw new Error('unsafe synthetic fixture cleanup');
    await rm(root, { recursive: true, force: true });
  }
}
test('all local pages and anchors pass without following public reference links', async () => {
  await fixture('<a href="second.html#anchor">local</a><a href="https://example.invalid/reference">reference</a>', async (root) => {
    const report = await validateSite(root, '/fixture');
    assert.equal(report.pages, 2); assert.equal(report.links, 1); assert.equal(report.external_requests, 0);
  });
});
test('broken local link fails before publication', async () => { await fixture('<a href="missing.html">broken</a>', (root) => assert.rejects(validateSite(root, '/fixture'), /broken-local-link/)); });
test('missing local anchor fails before publication', async () => { await fixture('<a href="second.html#missing">broken</a>', (root) => assert.rejects(validateSite(root, '/fixture'), /broken-anchor/)); });
test('active external resources are blocked and fail the gate', async () => { await fixture('<img src="https://example.invalid/never-contacted.png">', (root) => assert.rejects(validateSite(root, '/fixture'), /external-request/)); });
test('browser script errors fail the gate', async () => { await fixture('<script>throw new Error("synthetic fixture")</script>', (root) => assert.rejects(validateSite(root, '/fixture'), /browser-error/)); });
