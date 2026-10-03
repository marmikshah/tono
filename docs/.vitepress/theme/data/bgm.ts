import manifest from "../../../public/generated/sfx/bgm.json";
import type { SoundSample } from "./catalog";
import type { LibrarySound } from "./library";

export interface BackgroundMusic extends LibrarySound {
  bpm: number;
  key: string;
  bars: number;
}

interface ExportedMusic extends Omit<BackgroundMusic, "variants"> {
  variants: Omit<SoundSample, "template">[];
}

export const backgroundMusic: BackgroundMusic[] = (manifest as ExportedMusic[]).map(
  (track) => ({
    ...track,
    variants: track.variants.map((sample) => ({ ...sample, template: track.id, looping: true })),
  }),
);
