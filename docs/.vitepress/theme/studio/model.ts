export type InstrumentId = "sine" | "pulse" | "saw" | "fm" | "bell" | "pluck" | "bass" | "kick" | "snare" | "hat" | "noise" | "piano" | "flute" | "strings";
export interface Envelope { a: number; d: number; s: number; r: number }
export interface StudioLayer {
  id: string; name: string; instrument: InstrumentId; mute: boolean;
  gain: number; pan: number; transpose: number; env: Envelope;
  steps: string[]; noteLength: number; repeat: boolean; cutoff: number; delay: number;
}
export interface StudioProject {
  version: 1; name: string; tempo: number; duration: number; seed: number; loop: boolean; layers: StudioLayer[];
}
export const SAMPLE_RATE = 48_000;
export const MAX_LAYERS = 8;
export const DRAFT_KEY = "tono-sound-studio-v1";
export const instruments: { id: InstrumentId; name: string; detail: string; wave: string; drum?: number }[] = [
  { id: "sine", name: "Sine", detail: "Pure & soft", wave: "sine" },
  { id: "pulse", name: "Pulse", detail: "A little arcade", wave: "square" },
  { id: "saw", name: "Saw", detail: "Bright & buzzy", wave: "sawtooth" },
  { id: "fm", name: "FM keys", detail: "Warm & metallic", wave: "fm" },
  { id: "bell", name: "Bell", detail: "Clear & ringing", wave: "bell" },
  { id: "pluck", name: "Pluck", detail: "A little string", wave: "pluck" },
  { id: "bass", name: "Bass", detail: "Deep & rounded", wave: "bass" },
  { id: "kick", name: "Kick", detail: "The heartbeat", wave: "kit", drum: 36 },
  { id: "snare", name: "Snare", detail: "A crisp backbeat", wave: "kit", drum: 38 },
  { id: "hat", name: "Hi-hat", detail: "A little shimmer", wave: "kit", drum: 42 },
  { id: "noise", name: "Noise", detail: "Air & texture", wave: "noise" },
  { id: "piano", name: "Piano", detail: "Soft hammers", wave: "piano" },
  { id: "flute", name: "Flute", detail: "Light & airy", wave: "flute" },
  { id: "strings", name: "Strings", detail: "A slow bloom", wave: "strings" },
];
export const cloneProject = (project: StudioProject): StudioProject => JSON.parse(JSON.stringify(project));
export const instrumentFor = (id: InstrumentId) => instruments.find((instrument) => instrument.id === id)!;

export function parseNotes(text: string): number[] {
  const tokens = text.trim().split(/[\s+,]+/).filter(Boolean);
  if (tokens.length > 4) throw new Error("Use up to four notes in a chord.");
  return tokens.map((token) => {
    const drums: Record<string, number> = { kick: 36, snare: 38, hat: 42, "hi-hat": 42, clap: 39, "open-hat": 46 };
    if (Object.hasOwn(drums, token.toLowerCase())) return drums[token.toLowerCase()];
    const midi = /^midi:(\d{1,3})$/i.exec(token);
    if (midi && Number(midi[1]) <= 127) return Number(midi[1]);
    const note = /^([A-G])([#b]?)([0-8])$/i.exec(token);
    if (!note) throw new Error(`“${token}” needs a note like C4, F#3, or Kick.`);
    const offsets: Record<string, number> = { C: 0, D: 2, E: 4, F: 5, G: 7, A: 9, B: 11 };
    return (Number(note[3]) + 1) * 12 + offsets[note[1].toUpperCase()] + (note[2] === "#" ? 1 : note[2].toLowerCase() === "b" ? -1 : 0);
  });
}

export function createLayer(instrument: InstrumentId, id: string): StudioLayer {
  const definition = instrumentFor(instrument);
  const steps = Array<string>(16).fill("");
  const positions = instrument === "kick" ? [0, 8] : instrument === "snare" ? [4, 12] : instrument === "hat" ? [0, 2, 4, 6, 8, 10, 12, 14] : [0, 4, 8, 12];
  positions.forEach((step, index) => { steps[step] = definition.drum ? definition.name === "Hi-hat" ? "Hat" : definition.name : instrument === "bass" ? ["C2", "C2", "G1", "A#1"][index] : ["C4", "E4", "G4", "E4"][index]; });
  return { id, name: definition.name, instrument, mute: false, gain: definition.drum ? 0.45 : 0.35, pan: 0, transpose: 0,
    env: { a: 0.005, d: 0.08, s: definition.drum ? 1 : 0.45, r: 0.06 },
    steps, noteLength: definition.drum ? 1 : 2, repeat: true, cutoff: 20_000, delay: 0 };
}

export type StarterId = "chime" | "zap" | "beat" | "blank";
export function createStarter(id: StarterId = "chime"): StudioProject {
  const project: StudioProject = { version: 1, name: "Little chime", tempo: 120, duration: 2, seed: 42, loop: false, layers: [] };
  if (id === "blank") return { ...project, name: "My sound" };
  if (id === "chime") {
    const bell = createLayer("bell", "layer_1");
    bell.steps.fill(""); ["C5", "E5", "G5"].forEach((note, i) => { bell.steps[i * 3] = note; });
    bell.name = "Little bells"; bell.noteLength = 4; bell.repeat = false; bell.env = { a: 0.002, d: 0.25, s: 0.4, r: 0.12 };
    project.layers = [bell];
  } else if (id === "zap") {
    const pulse = createLayer("pulse", "layer_1");
    pulse.steps.fill(""); ["C7", "G6", "C5", "G3"].forEach((note, i) => { pulse.steps[i] = note; });
    pulse.name = "Falling pulse"; pulse.repeat = false; pulse.noteLength = 1; pulse.env = { a: 0.001, d: 0.035, s: 0.08, r: 0.015 }; pulse.cutoff = 9000;
    project.name = "Arcade zap"; project.tempo = 240; project.duration = 0.6; project.layers = [pulse];
  } else {
    project.name = "Small beat"; project.tempo = 110; project.duration = 480 / 110; project.loop = true;
    project.layers = [createLayer("kick", "layer_1"), createLayer("snare", "layer_2"), createLayer("hat", "layer_3"), createLayer("bass", "layer_4")];
    project.layers[0].steps[6] = "Kick";
    project.layers[2].gain = 0.2; project.layers[2].pan = 0.18;
    project.layers[3].gain = 0.3; project.layers[3].noteLength = 4; project.layers[3].cutoff = 2400;
  }
  return project;
}

function record(value: unknown, label: string): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error(`${label} is missing.`);
  return value as Record<string, unknown>;
}
function number(value: unknown, label: string, min: number, max: number, integer = false): number {
  if (typeof value !== "number" || !Number.isFinite(value) || value < min || value > max || (integer && !Number.isInteger(value))) throw new Error(`${label} must be between ${min} and ${max}.`);
  return value;
}
function text(value: unknown, label: string, max = 80): string {
  if (typeof value !== "string" || value.length > max) throw new Error(`${label} needs a short name.`);
  return value.trim();
}
function flag(value: unknown, label: string): boolean {
  if (typeof value !== "boolean") throw new Error(`${label} needs an on or off value.`);
  return value;
}
export function validateProject(input: unknown): StudioProject {
  const value = record(input, "Project");
  if (value.version !== 1) throw new Error("This project uses an unsupported studio version.");
  if (!Array.isArray(value.layers) || value.layers.length > MAX_LAYERS) throw new Error(`Keep the mix to ${MAX_LAYERS} layers.`);
  const ids = new Set<string>();
  const layers = value.layers.map((input, index): StudioLayer => {
    const layer = record(input, `Layer ${index + 1}`);
    const id = text(layer.id, "Layer ID", 40);
    if (!/^[a-z0-9_]+$/.test(id) || id === "master" || ids.has(id)) throw new Error("Every layer needs its own valid ID.");
    ids.add(id);
    if (!instruments.some((instrument) => instrument.id === layer.instrument)) throw new Error("Choose a supported studio instrument.");
    if (!Array.isArray(layer.steps) || layer.steps.length !== 16) throw new Error("Every pattern needs 16 steps.");
    const steps = layer.steps.map((value) => { const note = text(value, "Note", 80); parseNotes(note); return note; });
    const env = record(layer.env, "Envelope");
    return { id, name: text(layer.name, "Layer name"), instrument: layer.instrument as InstrumentId, mute: flag(layer.mute, "Mute"),
      gain: number(layer.gain, "Volume", 0, 1), pan: number(layer.pan, "Pan", -1, 1), transpose: number(layer.transpose, "Pitch", -24, 24, true),
      env: { a: number(env.a, "Attack", 0, 2), d: number(env.d, "Decay", 0, 2), s: number(env.s, "Sustain", 0, 1), r: number(env.r, "Release", 0, 2) },
      steps, noteLength: number(layer.noteLength, "Note length", 1, 16, true), repeat: flag(layer.repeat, "Repeat"),
      cutoff: number(layer.cutoff, "Brightness", 40, 20000), delay: number(layer.delay, "Echo", 0, 0.75) };
  });
  return { version: 1, name: text(value.name, "Sound name"), tempo: number(value.tempo, "Tempo", 40, 240), duration: number(value.duration, "Duration", 0.25, 30),
    seed: number(value.seed, "Seed", 0, 4294967295, true), loop: flag(value.loop, "Loop"), layers };
}

export interface StudioNote { step: number; len: number; pitch: string; gain: number }
export interface StudioNode { type: string; [key: string]: unknown }
export interface StudioDocument {
  name: string; duration: number; sample_rate: number; seed: number; version: number; engine: number;
  stereo: { mode: string }; playback: { mode: string; start_secs?: number; end_secs?: number; crossfade_secs?: number };
  root: { type: "tracks"; tracks: { id: string; node: StudioNode; pan: number; gain: number; mute: boolean }[]; master: StudioNode[] };
}
export function compileProject(input: StudioProject): StudioDocument {
  const project = validateProject(input);
  const targetFrames = Math.round(project.duration * SAMPLE_RATE);
  const fadeFrames = project.loop ? Math.min(480, SAMPLE_RATE * 30 - targetFrames) : 0;
  const duration = (targetFrames + fadeFrames) / SAMPLE_RATE;
  const stepSeconds = 60 / project.tempo / 4;
  const tracks = project.layers.map((layer) => {
    const notes: StudioNote[] = [];
    const limit = layer.repeat ? Math.ceil(duration / stepSeconds) : Math.min(16, Math.ceil(duration / stepSeconds));
    for (let step = 0; step < limit; step++) {
      for (const midi of parseNotes(layer.steps[step % 16])) {
        const pitch = midi + (instrumentFor(layer.instrument).drum ? 0 : layer.transpose);
        if (pitch < 0 || pitch > 127) throw new Error(`Bring ${layer.name || "this layer"} back into the note range.`);
        notes.push({ step, len: layer.noteLength, pitch: `midi:${pitch}`, gain: 0.8 });
      }
    }
    const voice = instrumentFor(layer.instrument);
    const node: StudioNode = notes.length ? { type: "seq", bpm: project.tempo, steps_per_beat: 4, wave: voice.wave, kit: "electronic", env: layer.env, notes } : { type: "sine", freq: 440 };
    const stages: StudioNode[] = [node];
    if (layer.cutoff < 20000) stages.push({ type: "lowpass", cutoff: layer.cutoff, q: 0.707 });
    if (layer.delay > 0) stages.push({ type: "delay", secs: layer.delay, feedback: 0.25 });
    return { id: layer.id, node: stages.length > 1 ? { type: "chain", stages } : node, gain: layer.gain, pan: layer.pan, mute: layer.mute || notes.length === 0 };
  });
  if (!tracks.length) tracks.push({ id: "silence", node: { type: "sine", freq: 440 }, gain: 0, pan: 0, mute: true });
  return { name: projectFilename(project), duration, sample_rate: SAMPLE_RATE, seed: project.seed, version: 2, engine: 5, stereo: { mode: "mono" },
    playback: project.loop ? { mode: "loop", start_secs: 0, end_secs: duration, crossfade_secs: fadeFrames / SAMPLE_RATE } : { mode: "oneshot" },
    root: { type: "tracks", tracks, master: [{ type: "gain", amount: 0.7 }] } };
}

export function exportProject(project: StudioProject): string {
  return JSON.stringify({ ...compileProject(project), _tono_studio: validateProject(project) });
}
export function importProject(source: string): StudioProject {
  if (source.length > 1_000_000) throw new Error("Choose a project file smaller than 1 MB.");
  let parsed: unknown;
  try { parsed = JSON.parse(source); } catch { throw new Error("This file needs valid project JSON."); }
  const value = record(parsed, "Project file");
  if (!value._tono_studio) throw new Error("Choose a project JSON saved from this sound studio.");
  return validateProject(value._tono_studio);
}
export const projectFilename = (project: StudioProject) => (project.name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || "my-sound");
