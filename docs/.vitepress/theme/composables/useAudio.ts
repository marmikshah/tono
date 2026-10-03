import { onBeforeUnmount, onMounted, reactive } from "vue";
import { sampleId, sampleUrl, type SoundSample } from "../data/catalog";
import { createRenderer } from "../audio/renderClient";
import { encodeWav, type RenderedAudio } from "../audio/wav";
import { documentKey, documentSource } from "../audio/document";

/** Shared transport for existing music files and locally rendered SoundDocs. */
export function useAudioPlayer() {
  const state = reactive({ activeId: "", playing: false, progress: 0, error: "", loadingId: "", downloadId: "" });
  let audio: HTMLAudioElement | undefined;
  let context: AudioContext | undefined;
  let renderer: ReturnType<typeof createRenderer> | undefined;
  let source: AudioBufferSourceNode | undefined;
  let buffer: AudioBuffer | undefined;
  let looping = false;
  let activeRenderKey = "";
  let offset = 0;
  let startedAt = 0;
  let mode: "media" | "synth" = "media";
  let request = 0;
  let animation = 0;
  let disposed = false;
  const downloads = new Map<string, ReturnType<typeof setTimeout>>();

  function position() {
    if (!buffer || !context) return 0;
    const elapsed = state.playing ? offset + context.currentTime - startedAt : offset;
    return looping ? elapsed % buffer.duration : Math.min(elapsed, buffer.duration);
  }

  function updateProgress() {
    state.progress = mode === "synth"
      ? (buffer ? position() / buffer.duration : 0)
      : (audio?.duration ? audio.currentTime / audio.duration : 0);
    if (state.playing) animation = requestAnimationFrame(updateProgress);
  }

  onMounted(() => {
    audio = new Audio();
    audio.preload = "none";
    audio.onplay = () => {
      if (mode !== "media") return;
      state.playing = !audio!.paused;
      cancelAnimationFrame(animation);
      updateProgress();
    };
    audio.onpause = () => {
      if (mode === "media" && audio?.paused) {
        state.playing = false;
        cancelAnimationFrame(animation);
      }
    };
    audio.onended = () => {
      if (mode !== "media") return;
      state.playing = false;
      state.progress = 1;
    };
  });

  function stopSource() {
    if (!source) return;
    source.onended = null;
    source.stop();
    source.disconnect();
    source = undefined;
  }

  function stop() {
    request++;
    audio?.pause();
    if (audio) audio.currentTime = 0;
    stopSource();
    offset = 0;
    cancelAnimationFrame(animation);
    state.playing = false;
    state.progress = 0;
    state.loadingId = "";
    state.error = "";
  }

  async function toggle(id: string, url: string) {
    if (!audio) return;
    if (mode === "media" && state.activeId === id && !audio.paused) {
      request++;
      audio.pause();
      cancelAnimationFrame(animation);
      updateProgress();
      state.playing = false;
      return;
    }
    const sameMedia = mode === "media" && state.activeId === id;
    if (!sameMedia) stop();
    mode = "media";
    const current = ++request;
    state.error = "";
    if (!sameMedia) {
      audio.src = url;
      state.activeId = id;
      state.progress = 0;
    } else if (audio.ended) audio.currentTime = 0;
    try {
      await audio.play();
      if (current === request) state.playing = !audio.paused;
    } catch {
      if (current === request) {
        state.playing = false;
        state.error = "This preview could not play. Please try again.";
      }
    }
  }

  async function toggleRendered(id: string, key: string, load: () => Promise<RenderedAudio>) {
    const sameSound = mode === "synth" && state.activeId === id && activeRenderKey === key;
    if (state.loadingId === id && sameSound) { stop(); return; }
    if (sameSound && state.playing) {
      request++;
      offset = position();
      stopSource();
      state.playing = false;
      cancelAnimationFrame(animation);
      updateProgress();
      return;
    }
    const resume = sameSound && buffer && state.progress < 1;
    if (!resume) { stop(); buffer = undefined; }
    mode = "synth";
    state.activeId = id;
    activeRenderKey = key;
    state.loadingId = id;
    state.error = "";
    const current = ++request;
    try {
      // Unlock on the user's click, before asynchronous fetching/rendering.
      context ??= new AudioContext();
      await context.resume();
      if (current !== request || disposed) return;
      if (!resume) {
        const rendered = await load();
        if (current !== request || disposed) return;
        buffer = context.createBuffer(2, rendered.left.length, rendered.sampleRate);
        buffer.copyToChannel(rendered.left, 0);
        buffer.copyToChannel(rendered.right, 1);
        looping = rendered.looping;
        offset = 0;
      }
      if (current !== request || disposed || !buffer) return;
      source = context.createBufferSource();
      source.buffer = buffer;
      source.loop = looping;
      source.connect(context.destination);
      source.onended = () => {
        if (current !== request) return;
        state.playing = false;
        state.progress = 1;
        offset = 0;
        source?.disconnect();
        source = undefined;
        cancelAnimationFrame(animation);
      };
      startedAt = context.currentTime;
      source.start(0, offset);
      state.playing = true;
      cancelAnimationFrame(animation);
      updateProgress();
    } catch (error) {
      if (current === request && !disposed) {
        state.playing = false;
        state.error = error instanceof Error ? error.message : "This preview could not play. Please try again.";
      }
    } finally {
      if (current === request) state.loadingId = "";
    }
  }

  function toggleSample(sample: SoundSample) {
    const url = sampleUrl(sample.source);
    return toggleRendered(sampleId(sample), `url:${url}`, () => {
      renderer ??= createRenderer();
      return renderer.render(url);
    });
  }

  async function toggleDocument(id: string, document: unknown) {
    try {
      const sourceJson = documentSource(document);
      await toggleRendered(id, documentKey(sourceJson), () => {
        renderer ??= createRenderer();
        return renderer.renderDocument(sourceJson);
      });
    } catch (error) {
      stop();
      state.error = error instanceof Error ? error.message : "This sound could not render.";
    }
  }

  async function downloadRendered(id: string, filename: string, load: () => Promise<RenderedAudio>) {
    if (state.downloadId) return;
    state.downloadId = id;
    state.error = "";
    try {
      const rendered = await load();
      if (disposed) return;
      const url = URL.createObjectURL(new Blob([encodeWav(rendered)], { type: "audio/wav" }));
      const link = document.createElement("a");
      link.href = url;
      link.download = filename;
      document.body.append(link);
      link.click();
      link.remove();
      downloads.set(url, setTimeout(() => { URL.revokeObjectURL(url); downloads.delete(url); }, 60_000));
    } catch (error) {
      if (!disposed) state.error = error instanceof Error ? error.message : "This sound could not download. Please try again.";
    } finally {
      state.downloadId = "";
    }
  }

  function downloadSample(sample: SoundSample) {
    return downloadRendered(sampleId(sample), sample.source.replace(/\.json$/, ".wav"), () => {
      renderer ??= createRenderer();
      return renderer.render(sampleUrl(sample.source));
    });
  }

  function downloadDocument(id: string, document: unknown, filename: string) {
    // Capture the current graph before any asynchronous work or further edits.
    let sourceJson: string;
    try { sourceJson = documentSource(document); }
    catch (error) {
      state.error = error instanceof Error ? error.message : "This sound could not download.";
      return Promise.resolve();
    }
    return downloadRendered(id, filename.endsWith(".wav") ? filename : `${filename}.wav`, () => {
      renderer ??= createRenderer();
      return renderer.renderDocument(sourceJson);
    });
  }

  onBeforeUnmount(() => {
    disposed = true;
    stop();
    renderer?.dispose();
    void context?.close().catch(() => {});
    if (audio) {
      audio.onplay = audio.onpause = audio.onended = null;
      audio.removeAttribute("src");
      audio.load();
    }
    for (const [url, timer] of downloads) { clearTimeout(timer); URL.revokeObjectURL(url); }
    downloads.clear();
  });

  return { state, toggle, toggleSample, toggleDocument, downloadSample, downloadDocument, stop };
}

export type AudioPlayer = ReturnType<typeof useAudioPlayer>;
