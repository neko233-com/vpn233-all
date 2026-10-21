// Real browser + real server, synthetic inputs only. No reports or traces are
// persisted in the repository, and failed assertions never print credentials.
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { once } from 'node:events'
import { mkdtemp, writeFile, rm } from 'node:fs/promises'
import { createServer } from 'node:net'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { chromium, expect } from '@playwright/test'

const root = fileURLToPath(new URL('..', import.meta.url))
const repo = resolve(root, '..')
const binary = process.env.VPN233_UI_BINARY || join(repo, 'target', 'release', `vpn233-subscribe-server${process.platform === 'win32' ? '.exe' : ''}`)
const work = await mkdtemp(join(tmpdir(), 'vpn233-ui-'))
const probe = createServer()
probe.listen(0, '127.0.0.1')
await once(probe, 'listening')
const port = probe.address().port
await new Promise(done => probe.close(done))
const origin = `http://127.0.0.1:${port}`
const config = join(work, 'config.yaml')
const structured = JSON.stringify({ proxies: [
  { name: 'WireGuard synthetic', type: 'wireguard', ip: '192.0.2.2', 'private-key': Buffer.alloc(32).toString('base64'), peers: [{ server: 'example.invalid', port: 51820, 'public-key': Buffer.alloc(32, 1).toString('base64'), 'allowed-ips': ['0.0.0.0/0'] }] },
  { name: 'Mieru synthetic', type: 'mieru', server: 'example.invalid', 'port-range': '2090-2099', transport: 'TCP', username: 'change-me', password: 'change-me' },
] })
await writeFile(config, JSON.stringify({
  server: { host: '127.0.0.1', http_port: port, data_dir: join(work, 'data'), web_dir: join(root, 'dist'), admin_user: 'root', admin_password: 'change-me-ui-password' },
  subscriptions: [{ name: 'synthetic', links: ['trojan://change-me@example.invalid:443#Synthetic', structured], url: '' }],
}), { mode: 0o600 })
const fixtureEnv = Object.fromEntries(Object.entries(process.env).filter(([key]) => !/^VPN233_/i.test(key)))
const child = spawn(binary, ['--config', config], { cwd: work, windowsHide: true, stdio: 'ignore', env: { ...fixtureEnv, VPN233_CONFIG: config, VPN233_ADMIN_USER: 'root', VPN233_ADMIN_PASSWORD: 'change-me-ui-password' } })
let spawnFailed = false
child.on('error', () => { spawnFailed = true })
let browser
try {
  let ready = false
  for (let attempt = 0; attempt < 150; attempt++) {
    if (spawnFailed || child.exitCode !== null) throw new Error('UI fixture server failed to start; build the subscribe binary first')
    try { ready = (await fetch(`${origin}/api/v1/formats`, { signal: AbortSignal.timeout(500) })).ok } catch { /* startup */ }
    if (ready) break
    await new Promise(done => setTimeout(done, 100))
  }
  assert.ok(ready, 'UI fixture server did not become healthy')
  browser = await chromium.launch({ headless: true })
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 } })
  const outside = []
  const runtimeErrors = []
  await context.route('**/*', async route => {
    const target = new URL(route.request().url())
    if (target.origin === origin || target.protocol === 'data:') await route.continue()
    else { outside.push('external request'); await route.abort() }
  })
  const page = await context.newPage()
  page.on('pageerror', () => runtimeErrors.push('browser runtime error'))
  await page.addInitScript(() => localStorage.setItem('vpn233-admin-language', 'en'))
  await page.goto(origin)
  await expect(page.locator('.login-panel')).toBeVisible()
  assert.ok((await page.title()).includes('VPN233'), 'unexpected page identity')
  await page.locator('.login-form input').nth(0).fill('root')
  await page.locator('.login-form input').nth(1).fill('change-me-wrong-password')
  await page.locator('.login-form button[type=submit]').click()
  await expect(page.locator('.error-line')).toBeVisible()
  await page.locator('.login-form input').nth(1).fill('change-me-ui-password')
  await page.locator('.login-form button[type=submit]').click()
  await expect(page.locator('.app-shell')).toBeVisible()
  assert.ok(await page.locator('h1').innerText(), 'dashboard is blank')
  const storage = await page.evaluate(() => ({
    persistentAuth: Object.keys(localStorage).some(key => /auth|token|jwt/i.test(key)),
    sessionAuth: Object.keys(sessionStorage).some(key => /auth|token|jwt/i.test(key)),
  }))
  assert.ok(!storage.persistentAuth && storage.sessionAuth, 'authentication must be session scoped')
  const response = await page.request.get(`${origin}/api/v1/formats`)
  assert.ok(response.ok(), 'format discovery failed')
  await page.locator('.nav-item').nth(0).click()
  await expect(page.locator('.node-status-card').filter({ hasText: 'WireGuard synthetic' })).toBeVisible()
  await expect(page.locator('.node-status-card').filter({ hasText: 'Mieru synthetic' })).toContainText('example.invalid:2090-2099')
  await page.locator('.nav-item').nth(3).click()
  await page.locator('.page-heading .primary-button').click()
  const groupModal = page.locator('.modal')
  await groupModal.locator('input').nth(0).fill('ui-structured')
  await groupModal.locator('input').nth(1).fill('UI structured group')
  const wg = groupModal.locator('.node-option').filter({ hasText: 'WireGuard synthetic' }).locator('input')
  const mieru = groupModal.locator('.node-option').filter({ hasText: 'Mieru synthetic' }).locator('input')
  await wg.check()
  await expect(mieru).not.toBeChecked()
  await mieru.check()
  await wg.uncheck()
  await expect(mieru).toBeChecked()
  await wg.check()
  await groupModal.locator('button.primary-button').click()
  await expect(groupModal).not.toBeVisible()
  const groupRow = page.locator('tr').filter({ hasText: 'UI structured group' })
  await expect(groupRow.locator('td').nth(1)).toHaveText('2')
  await groupRow.locator('.table-action').first().click()
  await expect(wg).toBeChecked()
  await expect(mieru).toBeChecked()
  await expect(groupModal.locator('textarea')).toHaveValue('')
  await wg.uncheck()
  await groupModal.locator('button.primary-button').click()
  await expect(groupModal).not.toBeVisible()
  await expect(groupRow.locator('td').nth(1)).toHaveText('1')
  await groupRow.locator('.table-action').first().click()
  await expect(wg).not.toBeChecked()
  await expect(mieru).toBeChecked()
  await groupModal.locator('.modal-foot button[type=button]').click()
  await page.locator('.nav-item').nth(7).click()
  const mirrorSelect = page.locator('.deploy-panel .mirror-select select')
  await mirrorSelect.selectOption('')
  assert.ok((await page.locator('.deploy-panel pre').allTextContents()).every(command => /mirror 'off'/i.test(command)), 'direct downloads must disable mirrors')
  await mirrorSelect.selectOption('https://ghfast.top/')
  assert.ok((await page.locator('.deploy-panel pre').allTextContents()).every(command => /mirror 'https:\/\/ghfast.top\/'/i.test(command)), 'selected mirror must reach the installer')
  await page.locator('.nav-item').nth(5).click()
  await page.locator('.page-heading .primary-button').click()
  const modal = page.locator('.modal')
  await expect(modal).toBeVisible()
  await modal.locator('input').nth(0).fill('change-me-ui-token')
  await modal.locator('input').nth(1).fill('change-me-subscription-password')
  await modal.locator('input').nth(2).fill('UI synthetic profile')
  await modal.locator('button[type=submit]').click()
  await expect(modal.locator('.generated-link-input')).toBeVisible()
  const subscription = new URL(await modal.locator('.generated-link-input').inputValue())
  assert.ok(subscription.origin === origin && subscription.searchParams.has('token'), 'subscription creation returned an invalid local link')
  await expect(modal.locator('.qr-preview img')).toHaveAttribute('src', /^data:image\/png;/)
  await modal.locator('.modal-foot button[type=button]').click()
  assert.ok(!(await page.locator('.token-value').first().innerText()).includes('change-me-ui-token'), 'token must initially be masked')
  await page.reload()
  await expect(page.locator('.app-shell')).toBeVisible()
  await page.setViewportSize({ width: 390, height: 844 })
  await page.locator('.mobile-menu').click()
  await expect(page.locator('.sidebar')).toHaveClass(/open/)
  await page.locator('.nav-item').nth(5).click()
  await expect(page.locator('.sidebar')).not.toHaveClass(/open/)
  const fits = await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)
  assert.ok(fits, 'mobile page overflows the viewport')
  await page.locator('.mobile-menu').click()
  await page.locator('.operator button').click()
  await expect(page.locator('.login-panel')).toBeVisible()
  assert.ok(await page.evaluate(() => sessionStorage.length === 0), 'logout must clear authentication')
  assert.equal(outside.length, 0, 'UI contacted an external service')
  assert.equal(runtimeErrors.length, 0, 'UI raised a browser runtime error')
  console.log('UI passed: real login/rejection, session privacy, structured nodes/group selection and removal, mirror/direct commands, token/link/QR, reload, mobile navigation, logout, zero external requests')
} finally {
  if (browser) await browser.close()
  if (child.exitCode === null && !spawnFailed) {
    child.kill()
    await Promise.race([once(child, 'exit'), new Promise(done => setTimeout(done, 5000))])
  }
  await rm(work, { recursive: true, force: true })
}
