import assert from 'node:assert/strict';
import test from 'node:test';
import { encodeWav } from '../docs/.vitepress/theme/audio/wav.ts';

test('WAV contains actual stereo PCM, the source rate, and bounded signed samples', () => {
  const result = encodeWav({
    left: new Float32Array([0, 1, -1, 2, -2, 0.5]),
    right: new Float32Array([0.25, -0.25, 0.75, 0, 1, -0.5]),
    sampleRate: 44100, looping: false,
  });
  const view = new DataView(result);
  const text = (start, length) => new TextDecoder().decode(new Uint8Array(result, start, length));
  assert.equal(text(0, 4), 'RIFF');
  assert.equal(text(8, 4), 'WAVE');
  assert.equal(text(12, 4), 'fmt ');
  assert.equal(text(36, 4), 'data');
  assert.equal(view.getUint32(4, true), result.byteLength - 8);
  assert.equal(view.getUint16(20, true), 1);
  assert.equal(view.getUint16(22, true), 2);
  assert.equal(view.getUint32(24, true), 44100);
  assert.equal(view.getUint32(28, true), 176400);
  assert.equal(view.getUint16(32, true), 4);
  assert.equal(view.getUint16(34, true), 16);
  assert.equal(view.getUint32(40, true), 24);
  assert.deepEqual(Array.from({ length: 12 }, (_, i) => view.getInt16(44 + 2 * i, true)),
    [0, 8192, 32767, -8192, -32768, 24575, 32767, 0, -32768, 32767, 16384, -16384]);
});

test('loop downloads retain whole-buffer sampler loop metadata', () => {
  const result = encodeWav({ left: new Float32Array(8), right: new Float32Array(8), sampleRate: 48000, looping: true });
  const at = 44 + 8 * 4;
  const view = new DataView(result);
  assert.equal(new TextDecoder().decode(new Uint8Array(result, at, 4)), 'smpl');
  assert.equal(view.getUint32(at + 4, true), 60);
  assert.equal(view.getUint32(at + 16, true), 20833);
  assert.equal(view.getUint32(at + 36, true), 1);
  assert.equal(view.getUint32(at + 52, true), 0);
  assert.equal(view.getUint32(at + 56, true), 7);
  assert.equal(view.getUint32(4, true), result.byteLength - 8);
});

test('invalid PCM cannot produce a silently corrupt download', () => {
  const base = { left: new Float32Array([0]), right: new Float32Array([0]), sampleRate: 48000, looping: false };
  assert.throws(() => encodeWav({ ...base, right: new Float32Array(2) }));
  assert.throws(() => encodeWav({ ...base, sampleRate: 0 }));
  assert.throws(() => encodeWav({ ...base, left: new Float32Array([NaN]) }));
  assert.throws(() => encodeWav({ ...base, left: new Float32Array([Infinity]) }));
});
