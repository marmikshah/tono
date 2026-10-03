export interface RenderedAudio {
  left: Float32Array<ArrayBuffer>;
  right: Float32Array<ArrayBuffer>;
  sampleRate: number;
  looping: boolean;
}

/** A finished stereo bounce as interleaved 16-bit PCM in a RIFF/WAVE file. */
export function encodeWav(audio: RenderedAudio): ArrayBuffer {
  if (audio.left.length !== audio.right.length || audio.left.length === 0)
    throw new Error("Audio channels must contain the same number of frames.");
  if (!Number.isInteger(audio.sampleRate) || audio.sampleRate < 8000 || audio.sampleRate > 192000)
    throw new Error("Invalid audio sample rate.");
  const bytes = audio.left.length * 4;
  const buffer = new ArrayBuffer(44 + bytes + (audio.looping ? 68 : 0));
  const view = new DataView(buffer);
  const writeText = (offset: number, text: string) => {
    for (let i = 0; i < text.length; i++) view.setUint8(offset + i, text.charCodeAt(i));
  };
  writeText(0, "RIFF");
  view.setUint32(4, buffer.byteLength - 8, true);
  writeText(8, "WAVE");
  writeText(12, "fmt ");
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true);
  view.setUint16(22, 2, true);
  view.setUint32(24, audio.sampleRate, true);
  view.setUint32(28, audio.sampleRate * 4, true);
  view.setUint16(32, 4, true);
  view.setUint16(34, 16, true);
  writeText(36, "data");
  view.setUint32(40, bytes, true);
  for (let i = 0; i < audio.left.length; i++) {
    for (let channel = 0; channel < 2; channel++) {
      const value = (channel === 0 ? audio.left : audio.right)[i];
      if (!Number.isFinite(value)) throw new Error("Audio contains a non-finite sample.");
      const clipped = Math.max(-1, Math.min(1, value));
      view.setInt16(44 + i * 4 + channel * 2, Math.round(clipped * (clipped < 0 ? 32768 : 32767)), true);
    }
  }
  if (audio.looping) {
    // The renderer already crossfades the seamless loop body. Preserve its
    // whole-buffer loop intent for samplers/game engines that read smpl.
    const at = 44 + bytes;
    writeText(at, "smpl");
    view.setUint32(at + 4, 60, true);
    view.setUint32(at + 16, Math.round(1_000_000_000 / audio.sampleRate), true);
    view.setUint32(at + 20, 60, true);
    view.setUint32(at + 36, 1, true);
    view.setUint32(at + 56, audio.left.length - 1, true);
  }
  return buffer;
}
