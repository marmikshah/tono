import { onBeforeUnmount, onMounted, reactive } from "vue";
import { sampleId, sampleUrl, type SoundSample } from "../data/catalog";
import { createRenderer } from "../audio/renderClient";
import { encodeWav, type RenderedAudio } from "../audio/wav";
import { documentKey, documentSource } from "../audio/document";

// Keep the listener's volume choice when navigating between preview pages.
let previewVolume = 1;

/** Shared transport for existing music files and locally rendered SoundDocs. */
export function useAudioPlayer() {
  const state = reactive({
    activeId: "", playing: false, progress: 0, currentTime: 0, duration: 0,
    volume: previewVolume, looping: false, error: "", loadingId: "", downloadId: "",
  });
  let audio: HTMLAudioElement | undefined;
  let context: AudioContext | undefined;
  let output: GainNode | undefined;
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
  let selection: { kind: "media"; id: string; url: string }
    | { kind: "synth"; id: string; key: string; load: () => Promise<RenderedAudio> } | undefined;
  const downloads = new Map<string, ReturnType<typeof setTimeout>>();

  function position() {
    if (!buffer || !context || !buffer.duration) return 0;
    const elapsed = state.playing ? offset + context.currentTime - startedAt : offset;
    return looping ? elapsed % buffer.duration : Math.min(elapsed, buffer.duration);
  }

  function updateProgress() {
    const duration = mode === "synth" ? buffer?.duration : audio?.duration;
    state.duration = duration && Number.isFinite(duration) ? duration : 0;
    state.currentTime = mode === "synth" ? position() : (audio?.currentTime || 0);
    state.progress = state.duration ? state.currentTime / state.duration : 0;
    state.looping = mode === "synth" && Boolean(buffer) && looping;
    if (state.playing) animation = requestAnimationFrame(updateProgress);
  }

  onMounted(() => {
    audio = new Audio();
    audio.preload = "none";
    audio.volume = state.volume;
    audio.onloadedmetadata = audio.ondurationchange = audio.ontimeupdate = () => {
      if (mode !== "media") return;
      cancelAnimationFrame(animation);
      updateProgress();
    };
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
        updateProgress();
      }
    };
    audio.onended = () => {
      if (mode !== "media") return;
      state.playing = false;
      cancelAnimationFrame(animation);
      updateProgress();
    };
  });

  function stopSource() {
    if (!source) return;
    const previous = source;
    source = undefined;
    previous.onended = null;
    try { previous.stop(); } catch { /* A source that failed to start is already inactive. */ }
    previous.disconnect();
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
    state.currentTime = 0;
    state.loadingId = "";
    state.error = "";
  }

  async function toggle(id: string, url: string) {
    if (!audio || disposed) return;
    const sameMedia = mode === "media" && selection?.kind === "media" && selection.id === id && selection.url === url;
    if (sameMedia && !audio.paused) {
      request++;
      audio.pause();
      cancelAnimationFrame(animation);
      updateProgress();
      state.playing = false;
      return;
    }
    if (!sameMedia) stop();
    mode = "media";
    selection = { kind: "media", id, url };
    buffer = undefined;
    activeRenderKey = "";
    state.looping = false;
    const current = ++request;
    state.error = "";
    if (!sameMedia) {
      audio.src = url;
      state.activeId = id;
      state.progress = 0;
      state.currentTime = 0;
      state.duration = 0;
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

  function startSource(current: number) {
    if (!context || !buffer || !output) return;
    const node = context.createBufferSource();
    source = node;
    node.buffer = buffer;
    node.loop = looping;
    node.connect(output);
    node.onended = () => {
      if (current !== request || source !== node) return;
      state.playing = false;
      offset = buffer!.duration;
      node.disconnect();
      source = undefined;
      cancelAnimationFrame(animation);
      updateProgress();
    };
    startedAt = context.currentTime;
    node.start(0, offset);
    state.playing = true;
    cancelAnimationFrame(animation);
    updateProgress();
  }

  async function toggleRendered(id: string, key: string, load: () => Promise<RenderedAudio>) {
    if (disposed) return;
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
    if (!resume) { stop(); buffer = undefined; state.duration = 0; state.looping = false; }
    mode = "synth";
    selection = { kind: "synth", id, key, load };
    state.activeId = id;
    activeRenderKey = key;
    state.loadingId = id;
    state.error = "";
    const current = ++request;
    try {
      // Unlock on the user's click, before asynchronous fetching/rendering.
      context ??= new AudioContext();
      if (!output) {
        output = context.createGain();
        output.gain.value = state.volume;
        output.connect(context.destination);
      }
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
      startSource(current);
    } catch (error) {
      if (current === request && !disposed) {
        stopSource();
        state.playing = false;
        state.error = error instanceof Error ? error.message : "This preview could not play. Please try again.";
      }
    } finally {
      if (current === request) state.loadingId = "";
    }
  }

  function toggleActive() {
    if (!selection) return Promise.resolve();
    return selection.kind === "media"
      ? toggle(selection.id, selection.url)
      : toggleRendered(selection.id, selection.key, selection.load);
  }

  function seek(fraction: number) {
    if (!Number.isFinite(fraction) || state.loadingId || !state.duration || disposed) return;
    const progress = Math.min(1, Math.max(0, fraction));
    if (mode === "media") {
      if (audio) audio.currentTime = progress * state.duration;
    } else if (buffer) {
      const playing = state.playing;
      const current = ++request;
      stopSource();
      offset = looping && progress === 1 ? 0 : progress * buffer.duration;
      state.playing = false;
      if (playing && offset < buffer.duration) startSource(current);
    }
    cancelAnimationFrame(animation);
    updateProgress();
  }

  function setVolume(value: number) {
    if (!Number.isFinite(value)) return;
    state.volume = Math.min(1, Math.max(0, value));
    previewVolume = state.volume;
    if (audio) audio.volume = state.volume;
    if (context && output) {
      output.gain.cancelScheduledValues(context.currentTime);
      output.gain.setTargetAtTime(state.volume, context.currentTime, 0.01);
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
    output?.disconnect();
    void context?.close().catch(() => {});
    if (audio) {
      audio.onplay = audio.onpause = audio.onended = null;
      audio.onloadedmetadata = audio.ondurationchange = audio.ontimeupdate = null;
      audio.removeAttribute("src");
      audio.load();
    }
    for (const [url, timer] of downloads) { clearTimeout(timer); URL.revokeObjectURL(url); }
    downloads.clear();
  });

  return { state, toggle, toggleActive, toggleSample, toggleDocument, downloadSample, downloadDocument, seek, setVolume, stop };
}

export type AudioPlayer = ReturnType<typeof useAudioPlayer>;
