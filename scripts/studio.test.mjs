import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { cloneProject, compileProject, createLayer, createStarter, exportProject, importProject, instruments, parseNotes, validateProject } from '../docs/.vitepress/theme/studio/model.ts';

test('musical spelling, chords, and drum names resolve to playable MIDI notes', () => {
  assert.deepEqual(parseNotes('C4 E4 G4'), [60, 64, 67]);
  assert.deepEqual(parseNotes('F#3+Gb3, midi:36'), [54, 54, 36]);
  assert.deepEqual(parseNotes('Kick Snare Hat Clap'), [36, 38, 42, 39]);
  assert.deepEqual(parseNotes(''), []);
  for (const invalid of ['constructor', 'midi:128', 'H4', 'C4 E4 G4 B4 D5']) assert.throws(() => parseNotes(invalid));
});
test('saved JSON is a usable SoundDoc and restores every editable setting exactly', () => {
  const project = createStarter('beat');
  project.name = 'Quest: Victory / Beat'; project.seed = 4294967295;
  project.layers[3].pan = -0.6; project.layers[3].delay = 0.25;
  project.layers[0].mute = true;
  project.layers[3].steps[3] = 'C2 E2 G2';
  const source = exportProject(project);
  const document = JSON.parse(source);
  assert.equal(document.root.type, 'tracks');
  assert.equal(document.version, 2);
  assert.equal(document.engine, 5);
  assert.equal(document.name, 'quest-victory-beat');
  assert.deepEqual(importProject(source), project);
  assert.deepEqual(compileProject(importProject(source)), compileProject(project));
});
test('the largest editable project stays within its own import size limit', () => {
  const project = createStarter('blank'); project.duration = 30; project.tempo = 240;
  project.layers = Array.from({ length: 8 }, (_, i) => { const layer = createLayer('sine', `layer_${i}`); layer.steps.fill('C4 E4 G4 B4'); return layer; });
  const source = exportProject(project);
  assert.ok(source.length <= 1_000_000);
  assert.deepEqual(importProject(source), project);
});
test('imports reject oversized, malformed, incompatible, and unsafe settings', () => {
  assert.throws(() => importProject('x'.repeat(1_000_001)), /smaller than/);
  assert.throws(() => importProject('{invalid'), /valid project JSON/);
  assert.throws(() => importProject('{"root":{"type":"sine"}}'), /saved from this sound studio/);
  const invalid = [
    (project) => { project.duration = 31; },
    (project) => { project.seed = Number.MAX_SAFE_INTEGER; },
    (project) => { project.layers[0].pan = 2; },
    (project) => { project.layers[0].steps = ['C4']; },
    (project) => { project.layers[0].id = 'unsafe-id'; },
    (project) => { project.layers[0].instrument = 'sampler'; },
    (project) => { project.layers.push(cloneProject(project).layers[0]); },
    (project) => { project.layers = Array.from({ length: 9 }, (_, i) => createLayer('sine', `layer_${i}`)); },
  ];
  for (const corrupt of invalid) { const project = createStarter(); corrupt(project); assert.throws(() => validateProject(project)); }
});
test('pattern notes keep their step addresses, chord voices, transpose, and repeat intent', () => {
  const project = createStarter('blank'); project.duration = 4;
  const layer = createLayer('sine', 'lead');
  layer.steps.fill(''); layer.steps[2] = 'C4 E4'; layer.transpose = 12; layer.noteLength = 3;
  project.layers = [layer];
  const notes = compileProject(project).root.tracks[0].node.notes;
  assert.deepEqual(notes, [
    { step: 2, len: 3, pitch: 'midi:72', gain: 0.8 }, { step: 2, len: 3, pitch: 'midi:76', gain: 0.8 },
    { step: 18, len: 3, pitch: 'midi:72', gain: 0.8 }, { step: 18, len: 3, pitch: 'midi:76', gain: 0.8 },
  ]);
  layer.repeat = false;
  assert.equal(compileProject(project).root.tracks[0].node.notes.length, 2);
});

const binary = await readFile(new URL('../docs/public/generated/engine/tono.wasm', import.meta.url));
const { instance: { exports: wasm } } = await WebAssembly.instantiate(binary, {});
function render(project) {
  const bytes = new TextEncoder().encode(JSON.stringify(compileProject(project)));
  try {
    const pointer = wasm.tono_prepare(bytes.length);
    new Uint8Array(wasm.memory.buffer, pointer, bytes.length).set(bytes);
    if (wasm.tono_render() !== 1) throw new Error(new TextDecoder().decode(new Uint8Array(wasm.memory.buffer, wasm.tono_error_ptr(), wasm.tono_error_len())));
    return { frames: wasm.tono_frames(), rate: wasm.tono_sample_rate(), left: new Float32Array(wasm.memory.buffer, wasm.tono_left_ptr(), wasm.tono_frames()).slice() };
  } finally { wasm.tono_reset(); }
}
test('all studio voices and starter mixes produce actual finite audible WASM audio', () => {
  for (const instrument of instruments) {
    const project = createStarter('blank'); project.duration = 1;
    project.layers = [createLayer(instrument.id, 'voice')];
    const audio = render(project);
    assert.equal(audio.rate, 48000);
    assert.ok(audio.left.some((sample) => Math.abs(sample) > 0.00001), instrument.id);
    assert.ok(audio.left.every(Number.isFinite), instrument.id);
  }
  for (const id of ['chime', 'zap', 'beat']) assert.ok(render(createStarter(id)).left.some((sample) => Math.abs(sample) > 0.00001), id);
});
test('loop render has the requested period, including the 30-second boundary', () => {
  for (const duration of [0.25, 2, 480 / 110, 29.999, 30]) {
    const project = createStarter(); project.duration = duration; project.loop = true;
    const audio = render(project);
    assert.ok(Math.abs(audio.frames - Math.round(duration * audio.rate)) <= 1, `${duration}s: ${audio.frames} frames`);
    assert.ok(compileProject(project).duration <= 30);
  }
});
test('cleared patterns, empty projects and muted layers remain valid silent documents', () => {
  const project = createStarter();
  project.layers[0].mute = true;
  assert.ok(render(project).left.every((sample) => sample === 0));
  project.layers[0].mute = false; project.layers[0].steps.fill('');
  assert.ok(render(project).left.every((sample) => sample === 0));
  assert.ok(render(createStarter('blank')).left.every((sample) => sample === 0));
});
