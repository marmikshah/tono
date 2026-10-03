import manifest from "../../../public/generated/sfx/library.json";
import type { SoundSample } from "./catalog";

export interface LibrarySound {
  id: string;
  title: string;
  category: string;
  description: string;
  tags: string[];
  looping: boolean;
  bpm?: number;
  key?: string;
  bars?: number;
  variants: SoundSample[];
}

interface ExportedSound extends Omit<LibrarySound, "variants"> {
  variants: Omit<SoundSample, "template">[];
}

export const librarySounds: LibrarySound[] = (manifest as ExportedSound[]).map(
  (sound) => ({
    ...sound,
    variants: sound.variants.map((sample) => ({
      ...sample,
      template: sound.id,
      looping: sound.looping,
    })),
  }),
);
