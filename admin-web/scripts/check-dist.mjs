#!/usr/bin/env node
// Build artifact gate for the VPN233 admin console.
//
// It verifies the *built* admin UI only (never the sources):
//   1. dist/index.html exists and keeps the hardened <meta> policy (CSP + referrer).
//   2. dist/assets contains at least one JS and one CSS bundle, and no source maps
//      or inline scripts leak into the HTML.
//   3. The build contains no external resource references. Every HTML script/link
//      is a relative same-origin asset, and JS/CSS may only contain the library
//      identifiers documented in LIBRARY_URL_PREFIXES below (React/Redux/Immer
//      error-message and XML namespace URLs, emitted by dependencies and never
//      fetched). The two local dev origins in the CSP meta are explicitly allowed.
//   4. The main JS bundle stays below the 900 KB budget; compressed sizes are
//      reported for scripts and styles.
//
// Run with `npm run check` (normally via `npm run verify`).

import { readFile, readdir, stat } from 'node:fs/promises'
import { gzipSync } from 'node:zlib'
import { join, relative } from 'node:path'
import { fileURLToPath } from 'node:url'

const projectRoot = fileURLToPath(new URL('..', import.meta.url))
const distDir = join(projectRoot, 'dist')
const MAX_MAIN_JS_BYTES = 900 * 1024

// Origins that may appear inside the production Content-Security-Policy meta of
// index.html. They only permit the local development API (127.0.0.1 / localhost
// on a wildcard port) and are never external resources.
const LOCAL_CSP_ORIGINS = new Set(['http://127.0.0.1:*', 'http://localhost:*'])

// URLs embedded by production dependencies. They are identifiers printed in error
// messages or XML namespaces; the console never loads them. Keep this list short and
// explicit: anything else fails the check.
const LIBRARY_URL_PREFIXES = [
  'http://www.w3.org/', // XML / SVG / MathML namespaces emitted by react-dom
  'https://react.dev/', // React production error documentation links
  'https://redux.js.org/', // Redux error documentation (via recharts)
  'https://redux-toolkit.js.org/', // Redux Toolkit error documentation (via recharts)
  'https://bit.ly/', // Immer error documentation (via recharts)
]

const failures = []
const fail = message => failures.push(message)
const formatBytes = value => (value >= 1024 ? `${(value / 1024).toFixed(1)} KB` : `${value} B`)
const formatGzip = content => `${(gzipSync(content).length / 1024).toFixed(1)} KB gzip`

async function walk(dir) {
  const entries = await readdir(dir, { withFileTypes: true })
  const files = []
  for (const entry of entries) {
    const target = join(dir, entry.name)
    if (entry.isDirectory()) files.push(...await walk(target))
    else files.push(target)
  }
  return files
}

async function pathExists(target) {
  try { await stat(target); return true } catch { return false }
}

const toDistPath = file => relative(distDir, file).split('\\').join('/')

async function main() {
  if (!await pathExists(distDir)) {
    console.error('[check-dist] dist/ is missing. Run `npm run build` first.')
    process.exit(1)
  }

  const files = await walk(distDir)
  const htmlFiles = files.filter(file => file.endsWith('.html'))
  const jsFiles = files.filter(file => file.endsWith('.js'))
  const cssFiles = files.filter(file => file.endsWith('.css'))
  const textFiles = [...htmlFiles, ...jsFiles, ...cssFiles]
  const mapFiles = files.filter(file => file.endsWith('.map'))

  const indexPath = join(distDir, 'index.html')
  if (!await pathExists(indexPath)) fail('dist/index.html is missing')

  if (mapFiles.length) fail(`source maps must not be published: ${mapFiles.map(toDistPath).join(', ')}`)
  if (jsFiles.length < 1) fail('no JavaScript bundles found under dist/')
  if (cssFiles.length < 1) fail('no CSS bundles found under dist/')
  const assetFiles = files.filter(file => toDistPath(file).startsWith('assets/'))
  if (assetFiles.length < 2) fail(`expected at least 2 files under dist/assets, found ${assetFiles.length}`)

  // ---- index.html policy -------------------------------------------------
  let html = ''
  if (await pathExists(indexPath)) {
    html = await readFile(indexPath, 'utf8')
    const readMeta = pattern => html.match(pattern)?.[1] || ''
    const csp = readMeta(/<meta[^>]*http-equiv="Content-Security-Policy"[^>]*content="([^"]*)"[^>]*>/i)
    const referrer = readMeta(/<meta[^>]*name="referrer"[^>]*content="([^"]*)"[^>]*>/i)
    if (!csp) fail('index.html is missing the Content-Security-Policy meta tag')
    for (const directive of ["default-src 'self'", "style-src 'self' 'unsafe-inline'", "img-src 'self' data:", "connect-src 'self' http://127.0.0.1:* http://localhost:*", "frame-ancestors 'none'"]) {
      if (csp && !csp.includes(directive)) fail(`CSP meta is missing directive: ${directive}`)
    }
    if (referrer !== 'no-referrer') fail('index.html must set <meta name="referrer" content="no-referrer">')
    if (!html.includes('id="root"')) fail('index.html is missing the React root element')
    if (/<script(?![^>]*\bsrc=)[^>]*>/i.test(html)) fail('index.html must not contain inline <script> tags')
    if (/\son[a-z]+\s*=/i.test(html)) fail('index.html must not contain inline event handlers')

    const htmlRefs = [...html.matchAll(/(?:src|href)\s*=\s*["']([^"']+)["']/gi)].map(match => match[1].trim())
    for (const ref of htmlRefs) {
      if (!ref || ref.startsWith('#')) continue
      if (/^[a-z][a-z0-9+.-]*:/i.test(ref) || ref.startsWith('//')) {
        fail(`index.html references a non-relative resource: ${ref}`)
        continue
      }
      const assetPath = join(distDir, decodeURIComponent(ref.split(/[?#]/)[0]))
      if (!await pathExists(assetPath)) fail(`index.html references a missing asset: ${ref}`)
    }
    if (![...html.matchAll(/<script[^>]*\bsrc="([^"]+)"/gi)].some(match => match[1].startsWith('/assets/'))) {
      fail('index.html does not reference a built /assets/ script bundle')
    }
  }

  // ---- external references ----------------------------------------------
  const allowedLibraryUrls = new Set()
  const externalPattern = /https?:\/\/[^\s"'`<>)\\;]+/g
  for (const file of textFiles) {
    const content = await readFile(file, 'utf8')
    const distPath = toDistPath(file)
    if (/sourceMappingURL/.test(content)) fail(`${distPath} contains a sourceMappingURL comment`)
    for (const [url] of content.matchAll(externalPattern)) {
      if (distPath === 'index.html' && LOCAL_CSP_ORIGINS.has(url)) continue
      if (LIBRARY_URL_PREFIXES.some(prefix => url.startsWith(prefix))) { allowedLibraryUrls.add(url); continue }
      fail(`external URL in ${distPath}: ${url}`)
    }
  }

  // ---- bundle budget -----------------------------------------------------
  let mainJs = null
  for (const file of jsFiles) {
    if (!mainJs || (await stat(file)).size > (await stat(mainJs)).size) mainJs = file
  }
  let mainJsSize = 0
  if (mainJs) {
    const content = await readFile(mainJs)
    mainJsSize = content.length
    if (mainJsSize >= MAX_MAIN_JS_BYTES) {
      fail(`main JS bundle is ${formatBytes(mainJsSize)}, over the ${formatBytes(MAX_MAIN_JS_BYTES)} budget`)
    }
  }
  let cssSize = 0
  for (const file of cssFiles) cssSize += (await stat(file)).size
  const totalSize = (await Promise.all(files.map(file => stat(file)))).reduce((sum, info) => sum + info.size, 0)
  const indexSize = (await stat(indexPath)).size

  // ---- report ------------------------------------------------------------
  console.log('VPN233 admin-web dist check')
  console.log(`  dist:      ${relative(process.cwd(), distDir) || distDir}`)
  console.log(`  index:     index.html · ${formatBytes(indexSize)}`)
  console.log(`  assets:    ${assetFiles.length} files · ${jsFiles.length} js · ${cssFiles.length} css`)
  console.log(`  main js:   ${mainJs ? toDistPath(mainJs) : '-'} · ${formatBytes(mainJsSize)} raw (< ${formatBytes(MAX_MAIN_JS_BYTES)} budget) · ${mainJs ? formatGzip(await readFile(mainJs)) : '-'}`)
  console.log(`  styles:    ${formatBytes(cssSize)} raw total`)
  console.log(`  total:     ${formatBytes(totalSize)}`)
  console.log(`  privacy:   CSP meta · referrer no-referrer · no source maps · no inline scripts`)
  console.log(`  external:  0 resource references · ${allowedLibraryUrls.size} library identifier(s) allowlisted`)
  if (assetFiles.length < 2) console.log('  warning:   unexpected asset count')

  if (failures.length) {
    console.error('\n[check-dist] FAILED:')
    for (const message of failures) console.error(`  - ${message}`)
    process.exit(1)
  }
  console.log('\n[check-dist] OK')
}

main().catch(error => {
  console.error(`[check-dist] unexpected error: ${error.stack || error.message}`)
  process.exit(1)
})
