// Build the lean Rust browser renderer. The site ships this module and JSON
// sound recipes; PCM/WAV audio is produced only when a listener requests it.
import { spawnSync } from 'node:child_process'
import { mkdir, copyFile, stat } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'
import path from 'node:path'

const root = fileURLToPath(new URL('../', import.meta.url))
const result = spawnSync('cargo', [
  'build', '--locked', '--release', '--target', 'wasm32-unknown-unknown', '-p', 'tono-web',
], { cwd: root, stdio: 'inherit' })
if (result.error) throw result.error
if (result.status !== 0) {
  console.error('Install the Rust browser target with: rustup target add wasm32-unknown-unknown')
  process.exit(result.status ?? 1)
}
const destination = path.join(root, 'docs/public/generated/engine/tono.wasm')
const target = process.env.CARGO_TARGET_DIR || path.join(root, 'target')
await mkdir(path.dirname(destination), { recursive: true })
await copyFile(path.resolve(root, target, 'wasm32-unknown-unknown/release/tono_web.wasm'), destination)
console.log(`Prepared Tono browser engine (${Math.round((await stat(destination)).size / 1024)} KiB)`)
