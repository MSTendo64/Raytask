# RayTask for VS Code

Full language support for [RayTask](https://github.com/MSTendo64/Raytask) — a cross-platform programming language with one syntax for web, desktop, mobile, embedded, and systems code.

## Features

### Language Intelligence
- **Syntax highlighting** — Full TextMate grammar covering keywords, types, strings (interpolated/raw), numbers (hex/binary/float), comments, preprocessor directives, attributes, and operators
- **IntelliSense completions** — Keywords, built-in types, stdlib functions with snippet inserts (`DateTime.Parse`, `TimeSpan.FromSeconds`, `GC.Collect`, etc.)
- **Hover documentation** — Rich descriptions for 60+ keywords, types, and stdlib functions
- **Document outline** — Breadcrumb and outline view showing functions, classes, structs, and interfaces
- **Smart snippets** — 28 snippets including `class`, `struct`, `using`, `try/catch/finally`, `switch` with `when` guard, block lambdas, `DateTime`, `TimeSpan`, and more

### Diagnostics
- **`raytask check` on save** — Real-time error/warning squiggles via the compiler's type checker
- **On-change diagnostics** — Optional debounced checking while typing (enable `raytask.checkOnChange`)
- **Shadow copies** — Unsaved file diagnostics use temp copies so relative imports resolve correctly

### Commands
| Command | Description |
|---|---|
| `RayTask: Run` | Execute current file or project entry |
| `RayTask: Check File` | Run type checker on the active file |
| `RayTask: Build` | Compile to bytecode/native (configurable target) |
| `RayTask: Show AST` | Display the parsed abstract syntax tree |
| `RayTask: Restart Language Features` | Clear and re-run diagnostics |
| `RayTask: Initialize Project` | Scaffold a `project.rtp` with `src/main.rt` |
| `RayTask: Open project.rtp` | Open the project manifest |
| `RayTask: Install Package` | Install a dependency from the registry |
| `RayTask: Update Packages` | Update all dependencies |
| `RayTask: List Installed Packages` | Show installed packages |
| `RayTask: Search Packages` | Search the package registry |

### Debugger
- **Full DAP debugger** via `raytask dap`
- Breakpoints (line, conditional, logpoints with `{x}` interpolation)
- Continue / Step Over / Step In / Step Out / Pause
- Call stack with source file paths
- Variables view — named locals + globals with expandable arrays/objects/dicts
- Watch expressions and Debug Console evaluation
- Program `print()` output redirects to Debug Console
- Restart session support
- Auto-configuration for projects (reads `project.rtp` entry point)

### Package Manager Integration
- Context-aware commands (only activate in project workspaces)
- Install packages with optional version spec (`HttpClient@1.2.0`)
- Update, list, and search packages from the VS Code UI

### Status Bar
- Shows project name and version when in a project context
- Shows "RayTask Script" for standalone `.rt` files
- Click to open `project.rtp` or run the current file

## Install

1. Install the RayTask CLI:
   ```bash
   cargo install --path .
   ```

2. Build and install the VS Code extension:
   ```bash
   cd editors/vscode
   npm run package
   npm run install-local
   ```

3. Reload the VS Code window.

Or install from the VS Code Marketplace (when published):
   ```
   ext install raytask.raytask
   ```

## Settings

| Setting | Default | Description |
|---|---|---|
| `raytask.path` | `"raytask"` | Path to the CLI executable |
| `raytask.checkOnSave` | `true` | Run diagnostics on file save |
| `raytask.checkOnChange` | `false` | Debounced (500ms) diagnostics while typing |
| `raytask.trace.dap` | `false` | Log DAP traffic to output channel |
| `raytask.preferProjectEntry` | `true` | Run/build uses project entry point in project context |
| `raytask.buildTarget` | `"bytecode"` | Default build target: `bytecode`, `native`, `native-bin`, `app`, `wasm` |
| `raytask.hoverEnabled` | `true` | Show hover documentation |

## Debug

1. Open a `.rt` file.
2. Set breakpoints (click the gutter; right-click → Edit Breakpoint for conditions/log messages).
3. Press `F5` or **Run and Debug** → *RayTask: Debug current file*.

### Launch configuration

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

### Debug actions

| Action | Behavior |
|---|---|
| Continue (`F5`) | Run to next breakpoint or completion |
| Step Over (`F10`) | Next line in the same or shallower frame |
| Step In (`F11`) | Enter function calls |
| Step Out (`Shift+F11`) | Return to caller |
| Pause (`F6`) | Break while running |
| Variables | Locals and globals by name, expand objects |
| Watch / Debug Console | Type any variable or expression name |

## Tasks

Preconfigured tasks for `tasks.json`:

```json
{
  "version": "2.0.0",
  "tasks": [
    { "type": "raytask", "task": "build" },
    { "type": "raytask", "task": "run" },
    { "type": "raytask", "task": "check" },
    { "type": "raytask", "task": "test" }
  ]
}
```

## Notes

- Rebuild/reinstall the CLI after compiler updates: `cargo install --path .`
- Diagnostics use shadow copies for unsaved files so project-local imports resolve correctly
- RTBC format is **v7** (includes local debug ranges); apps with older stubs need a rebuild
- Multi-file imports share line numbers; the launch file path is stamped on all chunks in a DAP session
- Extension auto-detects `project.rtp` by walking up to 10 directory levels; project context enables package manager commands and entry-point resolution
