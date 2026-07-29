const { spawn } = require("child_process");
const vscode = require("vscode");
const fs = require("fs");
const os = require("os");
const path = require("path");

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
    // Prefer checking from a temp file so unsaved buffer is included
    const tmp = path.join(
      os.tmpdir(),
      `raytask-check-${process.pid}-${Date.now()}.rt`
    );
    fs.writeFileSync(tmp, doc.getText(), "utf8");

    const child = spawn(raytaskPath, ["check", tmp], {
      cwd: path.dirname(file),
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
        fs.unlinkSync(tmp);
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
        fs.unlinkSync(tmp);
      } catch (_) {}
      const text = `${stdout}\n${stderr}`;
      if (output) {
        output.appendLine(`raytask check ${file}`);
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
