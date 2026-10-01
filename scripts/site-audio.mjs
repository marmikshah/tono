import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { resolve } from "node:path";

const root = fileURLToPath(new URL("../", import.meta.url));
const result = spawnSync(
    "cargo",
    [
        "run",
        "--locked",
        "--release",
        "-p",
        "tono",
        "--example",
        "site_samples",
        "--",
        resolve(root, "docs/public/generated/sfx"),
    ],
    { cwd: root, stdio: "inherit" },
);

if (result.error) {
    console.error(
        "Preparing site audio requires Rust and Cargo:",
        result.error.message,
    );
}
process.exit(result.status ?? 1);
