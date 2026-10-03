import { withBase } from "vitepress";
import type { RenderedAudio } from "./wav";
import { documentKey, documentSource } from "./document";

/** One lazily loaded DSP worker with a bounded, per-session PCM cache. */
export function createRenderer() {
  let worker: Worker | undefined;
  let disposed = false;
  let sequence = 0;
  let cacheBytes = 0;
  const cache = new Map<string, RenderedAudio>();
  const requests = new Map<string, Promise<RenderedAudio>>();
  const pending = new Map<number, {
    resolve(audio: RenderedAudio): void;
    reject(error: Error): void;
    timer: ReturnType<typeof setTimeout>;
  }>();
  const maxCacheBytes = 24 * 1024 * 1024;
  const maxCacheEntries = 64;

  function reset(message: string) {
    worker?.terminate();
    worker = undefined;
    for (const request of pending.values()) {
      clearTimeout(request.timer);
      request.reject(new Error(message));
    }
    pending.clear();
    requests.clear();
  }

  function getWorker() {
    if (!worker) {
      worker = new Worker(new URL("./render.worker.ts", import.meta.url), { type: "module" });
      worker.onmessage = ({ data }: MessageEvent<{ id: number; audio?: RenderedAudio; error?: string }>) => {
        const request = pending.get(data.id);
        if (!request) return;
        clearTimeout(request.timer);
        pending.delete(data.id);
        if (data.audio) request.resolve(data.audio);
        else request.reject(new Error(data.error || "This sound could not render."));
      };
      worker.onerror = () => reset("The sound engine stopped. Please try again.");
      worker.onmessageerror = () => reset("The sound engine returned unreadable audio. Please try again.");
    }
    return worker;
  }

  async function renderRequest(key: string, input: { sourceUrl: string } | { sourceJson: string }): Promise<RenderedAudio> {
    if (disposed) throw new Error("The sound player has closed.");
    const cached = cache.get(key);
    if (cached) {
      cache.delete(key);
      cache.set(key, cached);
      return cached;
    }
    const existing = requests.get(key);
    if (existing) return existing;
    const id = ++sequence;
    const request = new Promise<RenderedAudio>((resolve, reject) => {
      const currentWorker = getWorker();
      const timer = setTimeout(() => reset("Rendering took too long. Please try again."), 30_000);
      pending.set(id, { resolve, reject, timer });
      try {
        currentWorker.postMessage({ id, ...input, wasmUrl: withBase("/generated/engine/tono.wasm") });
      } catch (error) {
        clearTimeout(timer);
        pending.delete(id);
        reject(error instanceof Error ? error : new Error("This sound could not render."));
      }
    }).then((audio) => {
      if (disposed) throw new Error("The sound player has closed.");
      const bytes = audio.left.byteLength + audio.right.byteLength;
      while (cache.size && (cacheBytes + bytes > maxCacheBytes || cache.size >= maxCacheEntries)) {
        const oldest = cache.keys().next().value!;
        const oldAudio = cache.get(oldest)!;
        cacheBytes -= oldAudio.left.byteLength + oldAudio.right.byteLength;
        cache.delete(oldest);
      }
      if (bytes <= maxCacheBytes) { cache.set(key, audio); cacheBytes += bytes; }
      return audio;
    }).finally(() => { if (requests.get(key) === request) requests.delete(key); });
    requests.set(key, request);
    return request;
  }

  function dispose() {
    disposed = true;
    reset("The sound player has closed.");
    cache.clear();
    cacheBytes = 0;
    requests.clear();
  }
  function render(sourceUrl: string) {
    return renderRequest(`url:${sourceUrl}`, { sourceUrl });
  }

  function renderDocument(document: unknown) {
    const sourceJson = documentSource(document);
    return renderRequest(documentKey(sourceJson), { sourceJson });
  }

  return { render, renderDocument, dispose };
}
