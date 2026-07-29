"use strict";

/**
 * Project context detection.
 *
 * A "project" is a directory containing project.rtp.
 * Everything else is treated as a standalone script.
 */

const vscode = require("vscode");
const fs = require("fs");
const path = require("path");

/**
 * @typedef {{ kind: "project", root: string, rtp: string } | { kind: "script", file: string }} RtContext
 */

/**
 * Detect context for the given .rt document URI.
 * Walks up the directory tree looking for project.rtp.
 * @param {vscode.Uri} fileUri
 * @returns {RtContext}
 */
function detectContext(fileUri) {
  const filePath = fileUri.fsPath;
  let dir = path.dirname(filePath);

  // Walk up max 10 levels to find project.rtp
  for (let i = 0; i < 10; i++) {
    const candidate = path.join(dir, "project.rtp");
    if (fs.existsSync(candidate)) {
      return { kind: "project", root: dir, rtp: candidate };
    }
    const parent = path.dirname(dir);
    if (parent === dir) break; // filesystem root
    dir = parent;
  }

  return { kind: "script", file: filePath };
}

/**
 * Detect context for the active workspace folder (without requiring an open file).
 * Useful for commands that operate on the project root.
 * @returns {RtContext | null}
 */
function detectWorkspaceContext() {
  const folders = vscode.workspace.workspaceFolders;
  if (!folders || folders.length === 0) return null;

  for (const folder of folders) {
    const rtp = path.join(folder.uri.fsPath, "project.rtp");
    if (fs.existsSync(rtp)) {
      return { kind: "project", root: folder.uri.fsPath, rtp };
    }
  }
  return null;
}

/**
 * Read key-value pairs from project.rtp (simple heuristic, no full parse).
 * @param {string} rtpPath
 * @returns {{ name?: string, version?: string, entry?: string }}
 */
function readRtpMeta(rtpPath) {
  try {
    const src = fs.readFileSync(rtpPath, "utf8");
    const meta = {};
    // name: first quoted string after `project` / `package`
    const nameMatch = src.match(/^\s*(?:project|package)\s+"([^"]+)"/m);
    if (nameMatch) meta.name = nameMatch[1];
    // version = "..."
    const verMatch = src.match(/\bversion\s*=\s*"([^"]+)"/);
    if (verMatch) meta.version = verMatch[1];
    // entry = "..."
    const entryMatch = src.match(/\bentry\s*=\s*"([^"]+)"/);
    if (entryMatch) meta.entry = entryMatch[1];
    return meta;
  } catch {
    return {};
  }
}

module.exports = { detectContext, detectWorkspaceContext, readRtpMeta };
