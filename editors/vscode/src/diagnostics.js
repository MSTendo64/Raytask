const { spawn } = require("child_process");
const vscode = require("vscode");
const fs = require("fs");
const path = require("path");
const { detectContext } = require("./project");

/**
 * Run `raytask check` and parse `line:col: message` diagnostics.
 * @param {vscode.TextDocument} doc
 * @param {string} raytaskPath
 * @param {vscode.OutputChannel} [output]
 * @returns {Promise<vscode.Diagnostic[]>}
 */
function runDiagnostics(doc, raytaskPath, output) {
  return new Promise((resolve) => {
    const file = doc.uri.fsPath;
    const ctx = detectContext(doc.uri);
    const workDir = ctx.kind === "project" ? ctx.root : path.dirname(file);
    const useShadowCopy = doc.isDirty;
    const checkTarget = useShadowCopy
      ? path.join(
          path.dirname(file),
          `.${path.basename(file, ".rt")}.raytask-check-${process.pid}-${Date.now()}.rt`
        )
      : file;

    if (useShadowCopy) {
      fs.writeFileSync(checkTarget, doc.getText(), "utf8");
    }

    const child = spawn(raytaskPath, ["check", checkTarget], {
      cwd: workDir,
      shell: process.platform === "win32",
    });

    let stderr = "";
    let stdout = "";
    child.stdout.on("data", (d) => {
      stdout += d.toString();
    });
    child.stderr.on("data", (d) => {
      stderr += d.toString();
    });

    child.on("error", (err) => {
      try {
        if (useShadowCopy) fs.unlinkSync(checkTarget);
      } catch (_) {}
      const d = new vscode.Diagnostic(
        new vscode.Range(0, 0, 0, 1),
        `Cannot run raytask (${raytaskPath}): ${err.message}. Set raytask.path in settings.`,
        vscode.DiagnosticSeverity.Error
      );
      resolve([d]);
    });

    child.on("close", () => {
      try {
        if (useShadowCopy) fs.unlinkSync(checkTarget);
      } catch (_) {}
      const text = `${stdout}\n${stderr}`;
      if (output) {
        output.appendLine(`raytask check ${checkTarget}`);
        if (text.trim()) output.appendLine(text.trim());
      }
      resolve(parseDiagnostics(text, doc));
    });
  });
}

/**
 * @param {string} text
 * @param {vscode.TextDocument} doc
 */
function parseDiagnostics(text, doc) {
  /** @type {vscode.Diagnostic[]} */
  const out = [];
  const lines = text.split(/\r?\n/);
  // Formats:
  //   12:3: message
  //   error: 12:3: message
  //   FAILED ...
  const re = /(?:error:\s*)?(\d+):(\d+):\s*(.+)$/i;
  for (const line of lines) {
    const m = line.match(re);
    if (!m) continue;
    const ln = Math.max(0, parseInt(m[1], 10) - 1);
    const col = Math.max(0, parseInt(m[2], 10) - 1);
    const msg = m[3].trim();
    if (!msg || /^(OK|FAILED|note:)/i.test(msg)) continue;
    const endCol = Math.min(doc.lineAt(Math.min(ln, doc.lineCount - 1)).text.length, col + 40);
    const range = new vscode.Range(ln, col, ln, Math.max(col + 1, endCol));
    const severity = /warning/i.test(line)
      ? vscode.DiagnosticSeverity.Warning
      : vscode.DiagnosticSeverity.Error;
    out.push(new vscode.Diagnostic(range, msg, severity));
  }
  return out;
}

module.exports = { runDiagnostics, parseDiagnostics };
