const vscode = require("vscode");

const DOCS = {
  print: "Print values to stdout followed by a newline.\n\n`print(value)`",
  write: "Write values to stdout without a trailing newline.",
  readLine: "Read a line from stdin. Returns `string?`.",
  Main: "Program entry point. Must be `void Main()` or `async void Main()`.",
  var: "Local variable with type inference.",
  dyn: "Dynamically typed value (escapes static checking).",
  export: "Public visibility for types and members.",
  async: "Marks a function as asynchronous; returns a Task.",
  await: "Suspend until a Task completes.",
  owned: "Dispose the value when leaving the scope.",
  using: "Call Dispose at the end of the block.",
  stack: "Hint for stack allocation (MVP semantics).",
  unsafe: "Enable pointer operations (`ptr<T>`, `*`, `&`).",
  match: "Pattern match — Result Ok/Error arms supported.",
  List: "Growable array from `bstd.collections`.",
  Task: "`Task.Delay(ms)`, `Task.Run(fn)`, `Task.WhenAll(...)`.",
  Gc: "`Gc.Collect()`, `Gc.Stats()` — runtime GC controls.",
  Ok: "Construct a successful `Result` value.",
  Error: "Construct a failed `Result` value.",
  import: "Import a module namespace, e.g. `import bstd.io;`.",
};

/**
 * @param {vscode.TextDocument} document
 * @param {vscode.Position} position
 */
function provideHover(document, position) {
  const range = document.getWordRangeAtPosition(position);
  if (!range) return null;
  const word = document.getText(range);
  const doc = DOCS[word];
  if (!doc) return null;
  const md = new vscode.MarkdownString();
  md.appendCodeblock(word, "raytask");
  md.appendMarkdown("\n\n" + doc);
  return new vscode.Hover(md, range);
}

module.exports = { provideHover };
