# RayTask for VS Code

Language support and debugging for [RayTask](https://github.com/MSTendo64/Raytask).

## Features

- Syntax highlighting, snippets, outline
- Diagnostics via `raytask check` (on save)
- Completions / hover
- Commands: Run, Check, Build, Show AST
- **Full VM debugger** (`raytask dap`):
  - Breakpoints (per file), conditional breakpoints, logpoints (`{x}` in message)
  - Continue / Step Over / Step In / Step Out / Pause
  - Call stack with source paths
  - Named locals + globals; expand arrays / objects / dicts
  - Evaluate / watch by variable name
  - Program `print` → Debug Console (does not break DAP)
  - Restart session

## Install

Requires the RayTask CLI on `PATH` (or set `raytask.path`):

```bash
cargo install --path .
cd editors/vscode
npx @vscode/vsce package --no-dependencies -o raytask-0.1.1.vsix
code --install-extension ./raytask-0.1.1.vsix
```

Reload the window after install.

## Debug

1. Open a `.rt` file (e.g. `examples/interp.rt`).
2. Set breakpoints in the gutter (optional: right‑click → Edit Breakpoint for condition / log).
3. **Run and Debug** → *RayTask: Debug current file*, or `F5`.

```json
{
  "type": "raytask",
  "request": "launch",
  "name": "RayTask: Debug current file",
  "program": "${file}",
  "cwd": "${workspaceFolder}",
  "stopOnEntry": true,
  "raytaskPath": "raytask"
}
```

| Action | Behavior |
|--------|----------|
| Continue | Run to next breakpoint |
| Step Over | Next line in the same/shallower frame |
| Step In | Enter calls / next line |
| Step Out | Return to caller |
| Pause | Break while running |
| Variables | Locals by name (`x`, `y`, …) and Globals |
| Watch / Debug Console | Type a variable name |

## Settings

| Setting | Default | Description |
|---------|---------|-------------|
| `raytask.path` | `raytask` | Path to the CLI |
| `raytask.checkOnSave` | `true` | Diagnostics on save |
| `raytask.checkOnChange` | `false` | Debounced check while typing |
| `raytask.trace.dap` | `false` | Log DAP launch line |

## Notes

- Rebuild/reinstall the CLI after pulling debugger changes (`cargo install --path .`).
- RTBC format is **v7** (includes local debug ranges). Apps built with older stubs need a rebuild.
- Multi-file imports share line numbers today unless chunks carry `source`; the launch file path is stamped on all chunks in a DAP session.
