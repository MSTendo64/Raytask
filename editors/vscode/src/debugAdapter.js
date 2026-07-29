#!/usr/bin/env node
/**
 * Thin Debug Adapter launcher: spawns `raytask dap` and bridges stdio.
 * Prefer the DebugAdapterExecutable factory in extension.js when available;
 * this file remains as a package.json fallback.
 */
const { spawn } = require("child_process");
const path = require("path");

function resolveRaytask() {
  // VS Code may pass launch config via env when using some adapters;
  // default to PATH / setting injected by parent.
  return process.env.RAYTASK_PATH || process.env.RAYTASK || "raytask";
}

const raytask = resolveRaytask();
const child = spawn(raytask, ["dap"], {
  stdio: ["pipe", "pipe", "inherit"],
  shell: process.platform === "win32",
  env: process.env,
});

process.stdin.pipe(child.stdin);
child.stdout.pipe(process.stdout);

child.on("error", (err) => {
  process.stderr.write(
    `Failed to start RayTask DAP ('${raytask} dap'): ${err.message}\n` +
      `Install the CLI (cargo install --path .) or set raytask.path / raytaskPath.\n`
  );
  process.exit(1);
});

child.on("exit", (code, signal) => {
  if (signal) process.kill(process.pid, signal);
  process.exit(code == null ? 1 : code);
});

process.on("SIGINT", () => child.kill("SIGINT"));
process.on("SIGTERM", () => child.kill("SIGTERM"));
