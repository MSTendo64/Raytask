"use strict";

const vscode = require("vscode");
const path = require("path");
const fs = require("fs");
const { runDiagnostics } = require("./diagnostics");
const { provideCompletions } = require("./completion");
const { provideHover } = require("./hover");
const { detectContext, detectWorkspaceContext, readRtpMeta } = require("./project");

/** @type {vscode.DiagnosticCollection} */
let diagnostics;
/** @type {NodeJS.Timeout | undefined} */
let changeTimer;
/** @type {vscode.OutputChannel} */
let output;
/** @type {vscode.StatusBarItem} */
let statusBar;

// ── Status bar ─────────────────────────────────────────────────────────────────

/**
 * Update the status bar to reflect the current file's context.
 * @param {vscode.TextDocument | undefined} doc
 */
function updateStatusBar(doc) {
  if (!doc || doc.languageId !== "raytask" || doc.uri.scheme !== "file") {
    statusBar.hide();
    return;
  }

  const ctx = detectContext(doc.uri);
  if (ctx.kind === "project") {
    const meta = readRtpMeta(ctx.rtp);
    const label = meta.name ? `$(package) ${meta.name}` : "$(package) RayTask Project";
    const version = meta.version ? ` v${meta.version}` : "";
    statusBar.text = label + version;
    statusBar.tooltip = `Project root: ${ctx.root}\nproject.rtp: ${ctx.rtp}\n\nClick to open project.rtp`;
    statusBar.command = "raytask.openProjectFile";
    statusBar.backgroundColor = undefined;
  } else {
    statusBar.text = "$(file-code) RayTask Script";
    statusBar.tooltip = `Standalone script — no project.rtp found\n${ctx.file}\n\nClick to run`;
    statusBar.command = "raytask.runFile";
    statusBar.backgroundColor = undefined;
  }
  statusBar.show();
}

// ── Activate ────────────────────────────────────────────────────────────────────

/** @param {vscode.ExtensionContext} context */
function activate(context) {
  output = vscode.window.createOutputChannel("RayTask");
  diagnostics = vscode.languages.createDiagnosticCollection("raytask");

  // Status bar (lower-left, priority 10 so it sits near the language mode button)
  statusBar = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 10);
  context.subscriptions.push(diagnostics, output, statusBar);

  // ── Language features ──────────────────────────────────────────────────────

  const selector = { language: "raytask", scheme: "file" };

  context.subscriptions.push(
    vscode.languages.registerCompletionItemProvider(
      selector,
      { provideCompletionItems: provideCompletions },
      ".",
      ":"
    )
  );

  context.subscriptions.push(
    vscode.languages.registerHoverProvider(selector, { provideHover })
  );

  context.subscriptions.push(
    vscode.languages.registerDocumentSymbolProvider(selector, {
      provideDocumentSymbols(doc) {
        const symbols = [];
        const re =
          /^\s*(export\s+)?(async\s+)?(class|struct|interface|void|int|string|bool|long|double|float|var|dyn|[A-Za-z_][\w]*)\s+([A-Za-z_][\w]*)/gm;
        const text = doc.getText();
        let m;
        while ((m = re.exec(text))) {
          const kindWord = m[3];
          const name = m[4];
          if (["if", "for", "while", "return", "new", "this"].includes(name)) continue;
          const pos = doc.positionAt(m.index);
          let kind = vscode.SymbolKind.Function;
          if (kindWord === "class") kind = vscode.SymbolKind.Class;
          else if (kindWord === "struct") kind = vscode.SymbolKind.Struct;
          else if (kindWord === "interface") kind = vscode.SymbolKind.Interface;
          symbols.push(
            new vscode.DocumentSymbol(
              name,
              kindWord,
              kind,
              new vscode.Range(pos, pos),
              new vscode.Range(pos, pos)
            )
          );
        }
        return symbols;
      },
    })
  );

  // ── Commands ───────────────────────────────────────────────────────────────

  context.subscriptions.push(
    // Run — context-aware: project entry or current file
    vscode.commands.registerCommand("raytask.runFile", () => runContextAware("run")),

    // Check — always checks the active file (per-file diagnostics)
    vscode.commands.registerCommand("raytask.checkFile", () => {
      const ed = vscode.window.activeTextEditor;
      if (ed) return checkDocument(ed.document);
    }),

    // Build — context-aware
    vscode.commands.registerCommand("raytask.buildFile", () => runContextAware("build")),

    // AST view (always per-file)
    vscode.commands.registerCommand("raytask.showAst", () => runFileCommand(["ast"])),

    // Restart diagnostics
    vscode.commands.registerCommand("raytask.restartServer", () => {
      diagnostics.clear();
      vscode.window.showInformationMessage("RayTask language features restarted.");
    }),

    // Open project.rtp in editor
    vscode.commands.registerCommand("raytask.openProjectFile", () => {
      const ed = vscode.window.activeTextEditor;
      const ctx = ed
        ? detectContext(ed.document.uri)
        : detectWorkspaceContext();
      if (ctx && ctx.kind === "project") {
        vscode.window.showTextDocument(vscode.Uri.file(ctx.rtp));
      } else {
        vscode.window.showWarningMessage("No project.rtp found for the current file.");
      }
    }),

    // Create a new project.rtp scaffold in the workspace folder
    vscode.commands.registerCommand("raytask.initProject", async () => {
      const folders = vscode.workspace.workspaceFolders;
      if (!folders || folders.length === 0) {
        vscode.window.showErrorMessage("Open a folder first (File → Open Folder).");
        return;
      }
      const root = folders[0].uri.fsPath;
      const rtp = path.join(root, "project.rtp");
      if (fs.existsSync(rtp)) {
        vscode.window.showInformationMessage("project.rtp already exists.");
        vscode.window.showTextDocument(vscode.Uri.file(rtp));
        return;
      }
      const name = path.basename(root);
      const content = [
        `project "${name}" {`,
        `    version = "0.1.0"`,
        `    author  = ""`,
        `    description = ""`,
        ``,
        `    entry = "src/main.rt"`,
        ``,
        `    dependencies {`,
        `    }`,
        ``,
        `    build {`,
        `        target = "bytecode"`,
        `        gc = true`,
        `    }`,
        `}`,
        ``,
      ].join("\n");
      fs.mkdirSync(path.join(root, "src"), { recursive: true });
      if (!fs.existsSync(path.join(root, "src", "main.rt"))) {
        fs.writeFileSync(path.join(root, "src", "main.rt"), 'func main() {\n    print_ln("Hello, RayTask!");\n}\n');
      }
      fs.writeFileSync(rtp, content, "utf8");
      const doc = await vscode.window.showTextDocument(vscode.Uri.file(rtp));
      updateStatusBar(doc.document);
      vscode.window.showInformationMessage(`Created project.rtp for "${name}".`);
    }),

    // Package manager commands — only valid in a project
    vscode.commands.registerCommand("raytask.installPackage", async () => {
      const ctx = activeProjectContext();
      if (!ctx) return;
      const pkg = await vscode.window.showInputBox({
        prompt: "Package name (and optional version, e.g. HttpClient or HttpClient@1.2.0)",
        placeHolder: "LibraryName or LibraryName@1.0.0",
      });
      if (!pkg) return;
      runInTerminal([getRaytaskPath(), "install", pkg], ctx.root);
    }),

    vscode.commands.registerCommand("raytask.updatePackages", () => {
      const ctx = activeProjectContext();
      if (!ctx) return;
      runInTerminal([getRaytaskPath(), "update"], ctx.root);
    }),

    vscode.commands.registerCommand("raytask.listPackages", () => {
      const ctx = activeProjectContext();
      if (!ctx) return;
      runInTerminal([getRaytaskPath(), "list"], ctx.root);
    }),

    vscode.commands.registerCommand("raytask.searchPackage", async () => {
      const query = await vscode.window.showInputBox({
        prompt: "Search packages",
        placeHolder: "e.g. http",
      });
      if (!query) return;
      const ctx = activeProjectContext() || { root: vscode.workspace.workspaceFolders?.[0]?.uri.fsPath || "." };
      runInTerminal([getRaytaskPath(), "search", query], ctx.root);
    })
  );

  // ── Debug ──────────────────────────────────────────────────────────────────

  context.subscriptions.push(
    vscode.debug.registerDebugConfigurationProvider("raytask", {
      resolveDebugConfiguration(_folder, config) {
        // Auto-fill empty launch config (F5 with no launch.json)
        if (!config.type && !config.request && !config.name) {
          const editor = vscode.window.activeTextEditor;
          if (editor && editor.document.languageId === "raytask") {
            const ctx = detectContext(editor.document.uri);
            config.type = "raytask";
            config.request = "launch";
            config.stopOnEntry = true;

            if (ctx.kind === "project") {
              const meta = readRtpMeta(ctx.rtp);
              const entry = meta.entry
                ? path.join(ctx.root, meta.entry)
                : path.join(ctx.root, "src", "main.rt");
              config.name = `RayTask: ${meta.name || "Project"}`;
              config.program = entry;
              config.cwd = ctx.root;
              config._isProject = true;
            } else {
              config.name = "RayTask: Debug current file";
              config.program = editor.document.uri.fsPath;
              config.cwd = path.dirname(editor.document.uri.fsPath);
            }
          }
        }

        if (!config.program) {
          const editor = vscode.window.activeTextEditor;
          if (editor && editor.document.languageId === "raytask") {
            config.program = editor.document.uri.fsPath;
          }
        }
        if (!config.raytaskPath) config.raytaskPath = getRaytaskPath();
        if (!config.cwd) {
          config.cwd = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
        }
        return config;
      },
    })
  );

  context.subscriptions.push(
    vscode.debug.registerDebugAdapterDescriptorFactory("raytask", {
      createDebugAdapterDescriptor(session) {
        const exe = session.configuration.raytaskPath || getRaytaskPath();
        if (vscode.workspace.getConfiguration("raytask").get("trace.dap", false)) {
          output.appendLine(`DAP: ${exe} dap`);
        }
        return new vscode.DebugAdapterExecutable(exe, ["dap"], {
          cwd: session.configuration.cwd || undefined,
          env: { RAYTASK_PATH: exe },
        });
      },
    })
  );

  // ── Tasks ──────────────────────────────────────────────────────────────────

  context.subscriptions.push(
    vscode.tasks.registerTaskProvider("raytask", {
      provideTasks() {
        const rt = getRaytaskPath();
        const folder = vscode.workspace.workspaceFolders?.[0];
        if (!folder) return [];

        const wsCtx = detectWorkspaceContext();
        const isProject = wsCtx !== null;

        const make = (name, args, cwd) =>
          new vscode.Task(
            { type: "raytask", task: name },
            folder,
            name,
            "raytask",
            new vscode.ShellExecution(rt, args, { cwd }),
            "$raytask"
          );

        if (isProject) {
          return [
            make("build (project)", ["build", wsCtx.rtp], wsCtx.root),
            make("run (project)", ["run", wsCtx.rtp], wsCtx.root),
            make("check", ["check", "${file}"], wsCtx.root),
            make("test", ["test"], wsCtx.root),
            make("install", ["install", "${input:packageName}"], wsCtx.root),
            make("update", ["update"], wsCtx.root),
          ];
        }

        return [
          make("check", ["check", "${file}"], folder.uri.fsPath),
          make("run", ["run", "${file}"], folder.uri.fsPath),
          make("build", ["build", "${file}"], folder.uri.fsPath),
          make("test", ["test"], folder.uri.fsPath),
        ];
      },
      resolveTask(task) {
        return task;
      },
    })
  );

  // ── Event listeners ────────────────────────────────────────────────────────

  context.subscriptions.push(
    vscode.window.onDidChangeActiveTextEditor((ed) => {
      updateStatusBar(ed?.document);
    }),
    vscode.workspace.onDidSaveTextDocument((doc) => {
      if (doc.languageId !== "raytask") return;
      if (vscode.workspace.getConfiguration("raytask").get("checkOnSave", true)) {
        checkDocument(doc);
      }
    }),
    vscode.workspace.onDidChangeTextDocument((e) => {
      if (e.document.languageId !== "raytask") return;
      if (!vscode.workspace.getConfiguration("raytask").get("checkOnChange", false)) return;
      clearTimeout(changeTimer);
      changeTimer = setTimeout(() => checkDocument(e.document), 500);
    }),
    vscode.workspace.onDidCloseTextDocument((doc) => {
      diagnostics.delete(doc.uri);
    }),
    // Watch for project.rtp appearing / disappearing
    vscode.workspace.createFileSystemWatcher("**/project.rtp")
  );
  const watcher = vscode.workspace.createFileSystemWatcher("**/project.rtp");
  context.subscriptions.push(watcher);
  watcher.onDidCreate(() => updateStatusBar(vscode.window.activeTextEditor?.document));
  watcher.onDidDelete(() => updateStatusBar(vscode.window.activeTextEditor?.document));
  watcher.onDidChange(() => updateStatusBar(vscode.window.activeTextEditor?.document));

  // Initial state
  updateStatusBar(vscode.window.activeTextEditor?.document);
  for (const doc of vscode.workspace.textDocuments) {
    if (doc.languageId === "raytask") checkDocument(doc);
  }

  output.appendLine("RayTask extension activated.");
}

// ── Helpers ────────────────────────────────────────────────────────────────────

/** @param {vscode.TextDocument} doc */
async function checkDocument(doc) {
  if (doc.languageId !== "raytask" || doc.uri.scheme !== "file") return;
  try {
    const diags = await runDiagnostics(doc, getRaytaskPath(), output);
    diagnostics.set(doc.uri, diags);
  } catch (e) {
    output.appendLine(`check failed: ${e}`);
  }
}

/**
 * Run raytask with the correct target depending on project vs. script context.
 * For "run"/"build": project → pass rtp path; script → pass current file.
 * @param {"run" | "build"} sub
 */
async function runContextAware(sub) {
  const ed = vscode.window.activeTextEditor;
  if (!ed || ed.document.languageId !== "raytask") {
    vscode.window.showWarningMessage("Open a .rt file first.");
    return;
  }
  await ed.document.save();

  const ctx = detectContext(ed.document.uri);
  const rt = getRaytaskPath();

  if (ctx.kind === "project") {
    runInTerminal([rt, sub, ctx.rtp], ctx.root);
  } else {
    runInTerminal([rt, sub, ctx.file], path.dirname(ctx.file));
  }
}

/**
 * Run raytask with the current file regardless of project context.
 * @param {string[]} sub  e.g. ["ast"]
 */
async function runFileCommand(sub) {
  const ed = vscode.window.activeTextEditor;
  if (!ed || ed.document.languageId !== "raytask") {
    vscode.window.showWarningMessage("Open a .rt file first.");
    return;
  }
  await ed.document.save();
  runInTerminal([getRaytaskPath(), ...sub, ed.document.uri.fsPath], path.dirname(ed.document.uri.fsPath));
}

/**
 * @param {string[]} argv  Full command + args array
 * @param {string} cwd
 */
function runInTerminal(argv, cwd) {
  const term =
    vscode.window.terminals.find((t) => t.name === "RayTask") ||
    vscode.window.createTerminal({ name: "RayTask", cwd });
  term.show();
  const q = (s) => (/\s/.test(s) ? `"${s}"` : s);
  term.sendText(argv.map(q).join(" "));
}

/**
 * Get the active project context (from active editor or workspace).
 * Shows a warning if no project is found.
 * @returns {{ root: string, rtp: string } | null}
 */
function activeProjectContext() {
  const ed = vscode.window.activeTextEditor;
  const ctx = ed
    ? detectContext(ed.document.uri)
    : detectWorkspaceContext();

  if (!ctx || ctx.kind !== "project") {
    vscode.window.showWarningMessage(
      'No project.rtp found. Run \u201cRayTask: Initialize Project\u201d to create one.'
    );
    return null;
  }
  return ctx;
}

function getRaytaskPath() {
  return vscode.workspace.getConfiguration("raytask").get("path", "raytask");
}

function deactivate() {
  clearTimeout(changeTimer);
}

module.exports = { activate, deactivate };
