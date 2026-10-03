interface TonoExports extends WebAssembly.Exports {
  memory: WebAssembly.Memory;
  tono_abi_version(): number;
  tono_engine_version(): number;
  tono_prepare(bytes: number): number;
  tono_render(): number;
  tono_frames(): number;
  tono_sample_rate(): number;
  tono_left_ptr(): number;
  tono_right_ptr(): number;
  tono_error_ptr(): number;
  tono_error_len(): number;
  tono_reset(): void;
}

type RenderRequest = { id: number; wasmUrl: string } & (
  { sourceUrl: string; sourceJson?: never } | { sourceJson: string; sourceUrl?: never }
);
const worker = self as unknown as {
  onmessage: ((event: MessageEvent<RenderRequest>) => void) | null;
  postMessage(value: unknown, transfers?: Transferable[]): void;
};
let engine: Promise<TonoExports> | undefined;

function loadEngine(url: string) {
  engine ??= (async () => {
    const response = await fetch(url);
    if (!response.ok) throw new Error("The sound engine could not load. Please reload and try again.");
    const { instance } = await WebAssembly.instantiate(await response.arrayBuffer(), {});
    const exports = instance.exports as TonoExports;
    if (exports.tono_abi_version() !== 1 || exports.tono_engine_version() !== 5)
      throw new Error("The sound engine is out of date. Please reload this page.");
    return exports;
  })().catch((error) => { engine = undefined; throw error; });
  return engine;
}

async function renderRequest(data: RenderRequest) {
  try {
    const [wasm, source] = await Promise.all([loadEngine(data.wasmUrl), (async () => {
      if (typeof data.sourceJson === "string") return data.sourceJson;
      const response = await fetch(data.sourceUrl);
      if (!response.ok) throw new Error("This sound recipe could not load. Please try again.");
      return response.text();
    })()]);
    const json = JSON.parse(source) as { playback?: { mode?: string } | string };
    const bytes = new TextEncoder().encode(source);
    const errorMessage = () => new TextDecoder().decode(
      new Uint8Array(wasm.memory.buffer, wasm.tono_error_ptr(), wasm.tono_error_len()),
    ) || "This sound could not render.";
    try {
      const input = wasm.tono_prepare(bytes.length);
      if (!input) throw new Error(errorMessage());
      new Uint8Array(wasm.memory.buffer, input, bytes.length).set(bytes);
      if (wasm.tono_render() !== 1) throw new Error(errorMessage());
      const frames = wasm.tono_frames();
      const sampleRate = wasm.tono_sample_rate();
      // Rendering can grow WASM memory. Copy from the new buffer before reset.
      const left = new Float32Array(wasm.memory.buffer, wasm.tono_left_ptr(), frames).slice();
      const right = new Float32Array(wasm.memory.buffer, wasm.tono_right_ptr(), frames).slice();
      const looping = json.playback === "loop" || (typeof json.playback === "object" && json.playback?.mode === "loop");
      worker.postMessage({ id: data.id, audio: { left, right, sampleRate, looping } }, [left.buffer, right.buffer]);
    } finally {
      wasm.tono_reset();
    }
  } catch (error) {
    worker.postMessage({ id: data.id, error: error instanceof Error ? error.message : "This sound could not render." });
  }
}

// The raw ABI owns one request. Queue RPCs through fetch, render, and cleanup
// so direct editor documents and downloaded recipes share its lifetime safely.
let queue = Promise.resolve();
worker.onmessage = ({ data }) => { queue = queue.then(() => renderRequest(data)); };
