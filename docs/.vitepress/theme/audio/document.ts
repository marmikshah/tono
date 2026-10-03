/** Preserve serialized JSON exactly, including seeds beyond JS integer precision. */
export function documentSource(document: unknown): string {
  const source = typeof document === "string" ? document : JSON.stringify(document);
  if (source === undefined || !source.length) throw new Error("Add an instrument before rendering a sound.");
  if (new TextEncoder().encode(source).byteLength > 1024 * 1024)
    throw new Error("This sound recipe is too large to render in the browser.");
  return source;
}

/** Exact content keys avoid stale PCM when an editor changes the same sound ID. */
export function documentKey(source: string): string {
  return `document:${source}`;
}
