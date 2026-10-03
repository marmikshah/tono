import assert from 'node:assert/strict';
import test from 'node:test';
import { documentKey, documentSource } from '../docs/.vitepress/theme/audio/document.ts';

test('serialized imported graphs preserve u64 seeds byte for byte', () => {
  const saved = '{"name":"saved", "seed":18446744073709551615,"root":{"type":"noise"}}';
  assert.equal(documentSource(saved), saved);
});

test('editor snapshots and cache keys change when the same graph ID is edited', () => {
  const graph = { name: 'studio', root: { type: 'sine', freq: 440 } };
  const first = documentSource(graph);
  graph.root.freq = 880;
  const edited = documentSource(graph);
  assert.notEqual(documentKey(first), documentKey(edited));
  assert.equal(JSON.parse(first).root.freq, 440);
  assert.equal(documentKey(edited), documentKey(documentSource(graph)));
  assert.notEqual(documentKey(edited), `url:${edited}`);
});

test('undefined, cyclic and oversized UTF-8 graphs fail before worker submission', () => {
  assert.throws(() => documentSource(undefined));
  const cyclic = {};
  cyclic.root = cyclic;
  assert.throws(() => documentSource(cyclic));
  assert.throws(() => documentSource('é'.repeat(600_000)), /too large/);
});
