import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import ts from 'typescript';

const source = await readFile(new URL('../docs/.vitepress/theme/composables/useAudio.ts', import.meta.url), 'utf8');
const code = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText
  .replace(/^import .*;\n/gm, '').replace('export function useAudioPlayer', 'function useAudioPlayer');

function playerHarness() {
  let unmount;
  let frameId = 0;
  const frames = new Map();
  const contexts = [], nodes = [], media = [];
  const renders = [];
  let render = async (document) => ({ left: new Float32Array(32).fill(0.25), right: new Float32Array(32).fill(0.25), sampleRate: 8, looping: document.includes('loop') });
  class MockAudio {
    paused = true;
    currentTime = 0;
    duration = NaN;
    ended = false;
    volume = 1;
    constructor() { media.push(this); }
    async play() { this.paused = false; this.onplay?.(); }
    pause() { this.paused = true; this.onpause?.(); }
    removeAttribute() {}
    load() {}
  }
  class MockContext {
    currentTime = 0;
    destination = {};
    closed = false;
    constructor() { contexts.push(this); }
    async resume() {}
    async close() { this.closed = true; }
    createGain() {
      const gain = { value: 1, cancelScheduledValues() {}, setTargetAtTime(value) { this.value = value; } };
      this.output = { gain, connect() {}, disconnect() {} };
      return this.output;
    }
    createBuffer(channels, length, sampleRate) {
      assert.equal(channels, 2);
      return { duration: length / sampleRate, copyToChannel() {} };
    }
    createBufferSource() {
      const node = {
        active: false,
        connect(output) { this.output = output; },
        disconnect() {},
        start(_, offset) { this.active = true; this.offset = offset; },
        stop() { this.active = false; this.onended?.(); },
        end() { this.active = false; this.onended?.(); },
      };
      nodes.push(node);
      return node;
    }
  }
  const renderer = {
    renderDocument(document) { renders.push(document); return render(document); },
    render(url) { renders.push(url); return render(url); },
    dispose() { this.disposed = true; },
  };
  const dependencies = {
    onMounted: (callback) => callback(), onBeforeUnmount: (callback) => { unmount = callback; }, reactive: (state) => state,
    sampleId: (sample) => sample.source, sampleUrl: (source) => source,
    createRenderer: () => renderer, encodeWav: () => { throw new Error('Unexpected export in transport test'); },
    documentSource: (document) => typeof document === 'string' ? document : JSON.stringify(document), documentKey: (source) => `document:${source}`,
    Audio: MockAudio, AudioContext: MockContext,
    requestAnimationFrame: (callback) => { const id = ++frameId; frames.set(id, callback); return id; },
    cancelAnimationFrame: (id) => frames.delete(id),
  };
  const factory = new Function(...Object.keys(dependencies), `${code}\nreturn useAudioPlayer;`)(...Object.values(dependencies));
  return {
    player: factory(), createAnotherPlayer: factory, contexts, nodes, media, renders, renderer,
    tick(seconds) {
      contexts.forEach((context) => { context.currentTime += seconds; });
      const callbacks = [...frames.values()]; frames.clear(); callbacks.forEach((callback) => callback());
    },
    setRender: (callback) => { render = callback; },
    unmount: () => unmount(),
  };
}

test('preview seeking preserves playback, pause, loop wrapping and stop restart', async () => {
  const { player, nodes, contexts, tick, renders } = playerHarness();
  await player.toggleDocument('loop', { name: 'loop' });
  assert.equal(player.state.duration, 4);
  assert.equal(player.state.looping, true);
  tick(1);
  assert.equal(player.state.currentTime, 1);
  const previousEnd = nodes.at(-1).onended;
  player.seek(0.75);
  previousEnd();
  assert.equal(player.state.playing, true, 'an old source ending must not stop the replacement');
  assert.equal(nodes.filter((node) => node.active).length, 1);
  assert.equal(nodes.at(-1).offset, 3);
  tick(2);
  assert.equal(player.state.currentTime, 1);
  await player.toggleActive();
  assert.equal(player.state.playing, false);
  assert.equal(nodes.filter((node) => node.active).length, 0);
  player.seek(0.5);
  assert.equal(player.state.currentTime, 2);
  assert.equal(player.state.playing, false);
  await player.toggleActive();
  assert.equal(nodes.at(-1).offset, 2);
  assert.equal(renders.length, 1, 'paused seeking must reuse the rendered graph');
  player.seek(1);
  assert.equal(player.state.currentTime, 0);
  assert.equal(player.state.playing, true);
  player.setVolume(0.25);
  assert.equal(contexts[0].output.gain.value, 0.25);
  assert.equal(nodes.at(-1).output, contexts[0].output);
  player.stop();
  assert.equal(player.state.currentTime, 0);
  assert.equal(player.state.progress, 0);
  assert.equal(player.state.volume, 0.25);
  await player.toggleActive();
  assert.equal(nodes.at(-1).offset, 0);
  assert.equal(renders.length, 1);
});

test('one-shot end and seeking to end retain accurate duration and replay from zero', async () => {
  const { player, nodes } = playerHarness();
  await player.toggleDocument('once', { name: 'once' });
  nodes.at(-1).end();
  assert.equal(player.state.currentTime, 4);
  assert.equal(player.state.progress, 1);
  assert.equal(player.state.playing, false);
  await player.toggleActive();
  assert.equal(nodes.at(-1).offset, 0);
  player.seek(1);
  assert.equal(player.state.playing, false);
  assert.equal(player.state.currentTime, 4);
  await player.toggleActive();
  assert.equal(nodes.at(-1).offset, 0);
});

test('stop cancels pending playback and changed documents with the same ID never reuse old audio', async () => {
  const harness = playerHarness();
  const { player, nodes, renders } = harness;
  let finish;
  harness.setRender(() => new Promise((resolve) => { finish = resolve; }));
  const pending = player.toggleDocument('editor', { pitch: 60 });
  await Promise.resolve();
  assert.equal(player.state.loadingId, 'editor');
  player.stop();
  finish({ left: new Float32Array(32), right: new Float32Array(32), sampleRate: 8, looping: false });
  await pending;
  assert.equal(nodes.length, 0);
  assert.equal(player.state.loadingId, '');
  harness.setRender(async () => ({ left: new Float32Array(16), right: new Float32Array(16), sampleRate: 8, looping: false }));
  await player.toggleDocument('editor', { pitch: 61 });
  assert.equal(player.state.duration, 2);
  assert.equal(renders.length, 2);
  harness.unmount();
  assert.equal(nodes.filter((node) => node.active).length, 0);
  assert.equal(harness.renderer.disposed, true);
  assert.equal(harness.contexts[0].closed, true);
});

test('media transport follows metadata and supports seek, volume, pause and replay', async () => {
  const { player, media } = playerHarness();
  player.setVolume(0.3);
  assert.equal(media[0].volume, 0.3);
  await player.toggle('track', '/preview.ogg');
  media[0].duration = 20;
  media[0].onloadedmetadata();
  assert.equal(player.state.duration, 20);
  assert.equal(player.state.looping, false);
  player.seek(0.6);
  assert.equal(media[0].currentTime, 12);
  assert.equal(player.state.currentTime, 12);
  assert.equal(player.state.playing, true);
  await player.toggleActive();
  player.seek(-1);
  assert.equal(player.state.currentTime, 0);
  assert.equal(player.state.playing, false);
  player.setVolume(5);
  assert.equal(media[0].volume, 1);
  player.setVolume(NaN);
  assert.equal(media[0].volume, 1);
  await player.toggleActive();
  assert.equal(player.state.playing, true);
  player.stop();
  assert.equal(media[0].paused, true);
  assert.equal(player.state.currentTime, 0);
});

test('volume choice follows the listener when a new page creates its transport', () => {
  const { player, createAnotherPlayer, media } = playerHarness();
  player.setVolume(0.4);
  const next = createAnotherPlayer();
  assert.equal(next.state.volume, 0.4);
  assert.equal(media.at(-1).volume, 0.4);
  assert.equal(next.state.playing, false);
});
