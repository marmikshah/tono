import { withBase } from "vitepress";
import manifest from "../../../public/generated/sfx/manifest.json";

export interface SoundSample {
  template: string;
  seed: number;
  duration: number;
  sampleRate: number;
  source: string;
  score?: string;
  waveform: number[];
  label?: string;
  looping?: boolean;
}

const templates = [
  {
    id: "coin",
    title: "Coin pickup",
    category: "Arcade",
    description: "A bright little reward.",
    color: "gold",
  },
  {
    id: "jump",
    title: "Jump",
    category: "Movement",
    description: "A spring in every step.",
    color: "green",
  },
  {
    id: "laser",
    title: "Laser",
    category: "Action",
    description: "A little science fiction.",
    color: "purple",
  },
  {
    id: "explosion",
    title: "Explosion",
    category: "Action",
    description: "Give the big moments weight.",
    color: "orange",
  },
  {
    id: "impact",
    title: "Impact",
    category: "Action",
    description: "A hit you can feel.",
    color: "pink",
  },
  {
    id: "footstep",
    title: "Footstep",
    category: "Movement",
    description: "Bring the ground to life.",
    color: "sand",
  },
  {
    id: "ui-confirm",
    title: "Confirm",
    category: "Interface",
    description: "A satisfying little yes.",
    color: "green",
  },
  {
    id: "ui-cancel",
    title: "Cancel",
    category: "Interface",
    description: "A softer change of plan.",
    color: "blue",
  },
];

export const sounds = templates.map((template) => {
  const takes = (manifest as SoundSample[]).filter(
    (sample) => sample.template === template.id,
  );
  if (takes.length !== 4)
    throw new Error(`Expected four generated previews for ${template.id}`);
  return { ...template, takes };
});
export type Sound = (typeof sounds)[number];
export const sampleUrl = (file: string) => withBase(`/generated/sfx/${file}`);
export const sampleId = (sample: SoundSample) =>
  `sfx:${sample.source}`;

export interface MusicTrack {
  id: string;
  title: string;
  category: "Game music" | "Sound studies" | "Full pieces";
  mood: string;
  description: string;
  art: "puzzle" | "fantasy" | "arcade" | "jazz" | "piano" | "rhythm";
  detail: string;
  source?: string;
}

export const tracks: MusicTrack[] = [
  {
    id: "puzzle-menu",
    title: "Puzzle menu",
    category: "Game music",
    mood: "Playful & curious",
    description: "A small marimba motif with a lot of personality.",
    art: "puzzle",
    detail: "112 BPM · C major",
    source: "puzzle_menu.py",
  },
  {
    id: "emerald-vale",
    title: "Emerald vale",
    category: "Game music",
    mood: "Fantasy & exploration",
    description: "Lilting flute and harp rolls for a world worth exploring.",
    art: "fantasy",
    detail: "92 BPM · G major",
    source: "emerald_vale.py",
  },
  {
    id: "neon-rush",
    title: "Neon rush",
    category: "Game music",
    mood: "Arcade & action",
    description: "Driving synths, an octave bass, and an open road.",
    art: "arcade",
    detail: "104 BPM · C minor",
    source: "neon_rush.py",
  },
  {
    id: "noir-lounge",
    title: "Noir lounge",
    category: "Full pieces",
    mood: "Late nights & jazz",
    description: "Walking bass, Rhodes, and a little rain on the window.",
    art: "jazz",
    detail: "92 BPM · D minor",
    source: "noir_lounge.py",
  },
  {
    id: "golden-hour",
    title: "Golden hour",
    category: "Full pieces",
    mood: "Warm & laid-back",
    description: "A produced groove with swing, melody, and room to breathe.",
    art: "rhythm",
    detail: "Original composition",
    source: "golden_hour.py",
  },
  {
    id: "evening-glade",
    title: "Evening glade",
    category: "Game music",
    mood: "Calm & atmospheric",
    description: "A soft background loop for quieter moments.",
    art: "fantasy",
    detail: "Background loop",
  },
  {
    id: "iron-gauntlet",
    title: "Iron gauntlet",
    category: "Game music",
    mood: "Boss battle",
    description: "A tense loop for the moment everything is on the line.",
    art: "arcade",
    detail: "Battle loop",
  },
  {
    id: "sunny-steps",
    title: "Sunny steps",
    category: "Game music",
    mood: "Bright & bouncy",
    description: "A cheerful idle-platformer loop.",
    art: "puzzle",
    detail: "Platformer loop",
  },
  {
    id: "monsoon-melody",
    title: "Monsoon melody",
    category: "Full pieces",
    mood: "Gentle & cinematic",
    description: "Flute, nylon guitar, and a half-time groove.",
    art: "jazz",
    detail: "Original composition",
    source: "monsoon_melody.py",
  },
  {
    id: "fur-elise",
    title: "Für Elise",
    category: "Sound studies",
    mood: "Piano study",
    description: "Beethoven’s bagatelle, rebuilt as a typed score.",
    art: "piano",
    detail: "Sampled grand piano",
    source: "fur_elise.py",
  },
  {
    id: "band-demo",
    title: "One little band",
    category: "Sound studies",
    mood: "Four voices, one groove",
    description: "An ensemble playing together on the stereo bus.",
    art: "rhythm",
    detail: "Instrument showcase",
  },
  {
    id: "deep-note",
    title: "Deep note",
    category: "Sound studies",
    mood: "Expansive & cinematic",
    description: "Scattered synth voices resolving into a five-octave chord.",
    art: "arcade",
    detail: "Synth study",
  },
  {
    id: "retro-coin",
    title: "Retro coin",
    category: "Sound studies",
    mood: "Arcade pickup",
    description: "A tiny grace note with a familiar arcade feeling.",
    art: "puzzle",
    detail: "Sound effect",
  },
  {
    id: "jump-8bit",
    title: "8-bit jump",
    category: "Sound studies",
    mood: "Arcade movement",
    description: "A quick, rising square-wave sweep.",
    art: "arcade",
    detail: "Sound effect",
  },
  {
    id: "waka",
    title: "Waka",
    category: "Sound studies",
    mood: "A classic chomp",
    description: "Alternating pitch slides drawn into the notes.",
    art: "puzzle",
    detail: "Sound effect",
  },
  {
    id: "nokia-tune",
    title: "A familiar ringtone",
    category: "Sound studies",
    mood: "Plucked melody",
    description: "Thirteen notes of Gran Vals on a synthesized pluck.",
    art: "piano",
    detail: "Melody study",
  },
];

export const musicUrl = (track: MusicTrack) =>
  withBase(`/audio/${track.id}.mp4`);
export const scoreUrl = (track: MusicTrack) =>
  track.source
    ? `https://github.com/marmikshah/tono/blob/master/crates/tono-py/examples/${track.source}`
    : withBase("/guides/songs");
