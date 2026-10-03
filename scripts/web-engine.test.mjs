// Verify the actual shipped WebAssembly module, including every library recipe.
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import path from 'node:path'
import { createStarter, createLayer, compileProject, instruments } from '../docs/.vitepress/theme/studio/model.ts'
import ts from 'typescript'

const root = fileURLToPath(new URL('../', import.meta.url))
const generated = path.join(root, 'docs/public/generated')
const binary = await readFile(path.join(generated, 'engine/tono.wasm'))
const module = await WebAssembly.compile(binary)
assert.deepEqual(WebAssembly.Module.imports(module), [], 'engine needs no native host imports')
const { exports: wasm } = await WebAssembly.instantiate(module, {})
assert.equal(wasm.tono_abi_version(), 1)
assert.equal(wasm.tono_engine_version(), 5)
const decoder = new TextDecoder()
const errorMessage = () => decoder.decode(new Uint8Array(
  wasm.memory.buffer, wasm.tono_error_ptr(), wasm.tono_error_len(),
))

function render(source) {
  const bytes = Buffer.from(typeof source === 'string' ? source : JSON.stringify(source))
  try {
    const pointer = wasm.tono_prepare(bytes.length)
    if (!pointer) throw new Error(errorMessage())
    new Uint8Array(wasm.memory.buffer, pointer, bytes.length).set(bytes)
    if (wasm.tono_render() !== 1) throw new Error(errorMessage())
    const frames = wasm.tono_frames()
    const sampleRate = wasm.tono_sample_rate()
    // Copy each view before reset and after any growth during DSP allocations.
    const left = new Float32Array(wasm.memory.buffer, wasm.tono_left_ptr(), frames).slice()
    const right = new Float32Array(wasm.memory.buffer, wasm.tono_right_ptr(), frames).slice()
    return { left, right, sampleRate }
  } finally {
    wasm.tono_reset()
    assert.equal(wasm.tono_frames(), 0)
    assert.equal(wasm.tono_left_ptr(), 0)
    assert.equal(wasm.tono_right_ptr(), 0)
    assert.equal(wasm.tono_error_len(), 0)
  }
}

function checkAudio(audio, name) {
  assert.equal(audio.left.length, audio.right.length, `${name}: channel lengths`)
  assert(audio.left.length > 0, `${name}: nonempty audio`)
  let peak = 0
  for (const channel of [audio.left, audio.right]) {
    for (const sample of channel) {
      if (!Number.isFinite(sample)) throw new Error(`${name}: non-finite sample`)
      peak = Math.max(peak, Math.abs(sample))
    }
  }
  assert(peak > 0.00001, `${name}: audible audio`)
  assert(peak <= 1.00001, `${name}: sample headroom`)
}

const reference = spawnSync('cargo', [
  'run', '--locked', '--quiet', '-p', 'tono-web', '--example', 'web_reference',
], { cwd: root, encoding: 'utf8', maxBuffer: 8 * 1024 * 1024 })
if (reference.error) throw reference.error
assert.equal(reference.status, 0, reference.stderr)
const fixtures = JSON.parse(reference.stdout)
for (const expected of fixtures) {
  const audio = render(expected.source)
  const name = expected.document.name
  checkAudio(audio, name)
  assert.equal(audio.sampleRate, expected.sampleRate, `${name}: native sample rate`)
  assert.deepEqual(Array.from(new Uint32Array(audio.left.buffer)), expected.leftBits, `${name}: native left sample bits`)
  assert.deepEqual(Array.from(new Uint32Array(audio.right.buffer)), expected.rightBits, `${name}: native right sample bits`)
  if (name === 'tracks' || name === 'wide') assert.notDeepEqual(audio.left, audio.right, `${name}: stereo image`)
  if (name === 'loop') assert(audio.left.length < Math.ceil(expected.document.duration * audio.sampleRate), 'loop crossfade trims source')
}

// Exercise the actual worker RPC implementation with browser globals and
// transferred-channel delivery simulated in Node. JSON fetched from a URL
// must reach Rust unchanged, even when its u64 seed exceeds JS precision.
const workerSource = await readFile(path.join(root, 'docs/.vitepress/theme/audio/render.worker.ts'), 'utf8')
const workerCode = ts.transpileModule(workerSource, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText
const largeSeed = fixtures.find((fixture) => fixture.document.name === 'u64-seed')
const messages = []
let finishRpc
const rpcDone = new Promise((resolve) => { finishRpc = resolve })
const workerRuntime = {
  onmessage: undefined,
  postMessage(message) { messages.push(message); if (messages.length === 4) finishRpc() },
}
const workerFetch = async (url) => {
  if (url === 'engine') return new Response(binary)
  if (url === 'saved-graph') return new Response(largeSeed.source)
  throw new Error(`Unexpected worker fetch: ${url}`)
}
new Function('self', 'fetch', workerCode)(workerRuntime, workerFetch)
workerRuntime.onmessage({ data: { id: 1, wasmUrl: 'engine', sourceJson: fixtures[0].source } })
workerRuntime.onmessage({ data: { id: 2, wasmUrl: 'engine', sourceUrl: 'saved-graph' } })
workerRuntime.onmessage({ data: { id: 3, wasmUrl: 'engine', sourceJson: '{broken' } })
workerRuntime.onmessage({ data: { id: 4, wasmUrl: 'engine', sourceJson: fixtures[0].source } })
const rpcTimer = setTimeout(() => { throw new Error('Worker RPC test timed out') }, 30_000)
await rpcDone.finally(() => clearTimeout(rpcTimer))
assert.deepEqual(messages.map((message) => message.id), [1, 2, 3, 4], 'worker serializes mixed request lifetimes')
assert.deepEqual(Array.from(new Uint32Array(messages[1].audio.left.buffer)), largeSeed.leftBits, 'URL worker preserves u64 seeds')
assert(messages[2].error, 'worker reports malformed direct JSON')
assert.deepEqual(messages[0].audio.left, messages[3].audio.left, 'worker recovers after a malformed direct graph')

const valid = { name: 'recovery', duration: 0.01, root: { type: 'sine', freq: 440 } }
assert.throws(() => render('{bad JSON'), /document JSON/)
assert.throws(() => render({ ...valid, engine: 0 }), /unsupported engine/)
assert.throws(() => render({ ...valid, version: 99 }), /unsupported document version/)
assert.throws(() => render({ ...valid, duration: 31 }), /duration limit/)
assert.throws(() => render({ ...valid, sample_rate: 96_000 }), /sample-rate limit/)
assert.throws(() => render({ ...valid, root: { type: 'seq', bpm: 120, wave: 'sampler', sf2: 'piano.sf2',
  env: { a: 0, d: 0, s: 1, r: 0 }, notes: [{ step: 0, len: 1, pitch: 'C4' }],
} }), /native file/)
assert.throws(() => render(' '.repeat(1024 * 1024 + 1)), /UTF-8 bytes/)
checkAudio(render(valid), 'recovery after failures')

const starters = JSON.parse(await readFile(path.join(generated, 'sfx/manifest.json'), 'utf8'))
const library = JSON.parse(await readFile(path.join(generated, 'sfx/library.json'), 'utf8'))
const bgm = JSON.parse(await readFile(path.join(generated, 'sfx/bgm.json'), 'utf8'))
const manifest = [...starters, ...library.flatMap((entry) => entry.variants),
  ...bgm.flatMap((track) => track.variants.map((entry) => ({ ...entry, music: track })))]
let loopCount = 0
for (const entry of manifest) {
  const source = await readFile(path.join(generated, 'sfx', entry.source), 'utf8')
  const doc = JSON.parse(source)
  const audio = render(source)
  checkAudio(audio, entry.source)
  assert.equal(audio.sampleRate, entry.sampleRate, `${entry.source}: declared sample rate`)
  assert(Math.abs(audio.left.length / audio.sampleRate - entry.duration) < 1 / audio.sampleRate + 0.000001, `${entry.source}: declared duration`)
  if (doc.playback?.mode === 'loop') {
    loopCount++
    assert(audio.left.length < Math.ceil(doc.duration * audio.sampleRate), `${entry.source}: seamless loop length`)
  }
  if (entry.music) {
    const frames = Math.round(entry.music.bars * 4 * 60 / entry.music.bpm * audio.sampleRate)
    assert.equal(audio.left.length, frames, `${entry.source}: exact four-bar frame count`)
    for (const channel of [audio.left, audio.right]) {
      const seam = Math.abs(channel[0] - channel.at(-1))
      const seamDb = seam > 0 ? 20 * Math.log10(seam) : -Infinity
      assert(seamDb < -40, `${entry.source}: quiet stereo seam (${seamDb.toFixed(2)} dB)`)
    }
  }
  const replay = render(source)
  assert.deepEqual(audio.left, replay.left, `${entry.source}: deterministic left channel`)
  assert.deepEqual(audio.right, replay.right, `${entry.source}: deterministic right channel`)
}
console.log(`Verified ${fixtures.length} native/WASM equivalence fixtures and ${manifest.length} deterministic browser sounds (${loopCount} loops)`)

for (const starter of ['chime', 'zap', 'beat', 'blank']) {
  const project = createStarter(starter)
  const audio = render(compileProject(project))
  if (starter !== 'blank') checkAudio(audio, `studio ${starter}`)
  else assert(audio.left.every((sample) => sample === 0), 'blank studio is silent')
}
for (const instrument of instruments) {
  const project = createStarter('blank')
  project.duration = 0.5
  project.layers = [createLayer(instrument.id, 'voice')]
  const firstNote = project.layers[0].steps.find(Boolean)
  project.layers[0].steps.fill('')
  project.layers[0].steps[0] = firstNote
  const audio = render(compileProject(project))
  checkAudio(audio, `studio ${instrument.id}`)
}
const edited = createStarter('chime')
const beforeEdit = render(compileProject(edited))
edited.layers[0].transpose = 12
const afterEdit = render(compileProject(edited))
assert.notDeepEqual(beforeEdit.left, afterEdit.left, 'editing the same studio ID changes PCM')
console.log(`Verified 4 studio starters, ${instruments.length} studio instruments and edited graph rendering`)
