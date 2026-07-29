const vscode = require("vscode");

const KEYWORDS = [
  "import", "namespace", "module", "export", "protected", "private",
  "class", "struct", "interface", "abstract", "virtual", "override",
  "new", "base", "this", "super", "return", "if", "else", "switch", "case",
  "default", "break", "continue", "for", "foreach", "while", "do", "in",
  "try", "catch", "finally", "throw", "using", "unsafe", "stack", "owned",
  "async", "await", "match", "var", "dyn", "const", "property", "get", "set",
  "where", "operator", "params", "is", "as", "typeof", "sizeof",
];

const TYPES = [
  "void", "bool", "byte", "sbyte", "short", "ushort", "int", "uint",
  "long", "ulong", "float", "double", "decimal", "char", "string", "ptr",
  "List", "Dictionary", "Set", "Queue", "Stack", "Task", "Result",
  "DateTime", "File", "Directory", "StringBuilder", "Logger", "Http",
  "Json", "Yaml", "Math", "Random", "Hash", "Gc", "Object",
];

const STDLIB = [
  { label: "print", detail: "bstd.io", insert: "print($0);" },
  { label: "write", detail: "bstd.io", insert: "write($0);" },
  { label: "readLine", detail: "bstd.io", insert: "readLine()" },
  { label: "assert", detail: "bstd.test", insert: "assert($0);" },
  { label: "assertEq", detail: "bstd.test", insert: "assertEq($1, $2);" },
  { label: "Task.Delay", detail: "bstd.async", insert: "Task.Delay($0)" },
  { label: "Task.Run", detail: "bstd.async", insert: "Task.Run($0)" },
  { label: "Gc.Collect", detail: "runtime", insert: "Gc.Collect()" },
  { label: "File.ReadText", detail: "bstd.fs", insert: "File.ReadText($0)" },
  { label: "File.WriteText", detail: "bstd.fs", insert: "File.WriteText($1, $2)" },
  { label: "Json.Parse", detail: "bstd.json", insert: "Json.Parse($0)" },
  { label: "Json.Stringify", detail: "bstd.json", insert: "Json.Stringify($0)" },
  { label: "Ok", detail: "bstd.result", insert: "Ok($0)" },
  { label: "Error", detail: "bstd.result", insert: "Error($0)" },
];

const IMPORTS = [
  "bstd.io", "bstd.fs", "bstd.net", "bstd.async", "bstd.string",
  "bstd.regex", "bstd.json", "bstd.yml", "bstd.collections", "bstd.math",
  "bstd.time", "bstd.crypto", "bstd.unsafe", "bstd.result", "bstd.test",
  "bstd.logging",
];

/**
 * @param {vscode.TextDocument} document
 * @param {vscode.Position} position
 */
function provideCompletions(document, position) {
  const line = document.lineAt(position).text;
  const prefix = line.slice(0, position.character);
  /** @type {vscode.CompletionItem[]} */
  const items = [];

  if (/import\s+[\w.]*$/.test(prefix)) {
    for (const imp of IMPORTS) {
      const item = new vscode.CompletionItem(imp, vscode.CompletionItemKind.Module);
      item.insertText = imp;
      items.push(item);
    }
    return items;
  }

  for (const kw of KEYWORDS) {
    const item = new vscode.CompletionItem(kw, vscode.CompletionItemKind.Keyword);
    items.push(item);
  }
  for (const t of TYPES) {
    items.push(new vscode.CompletionItem(t, vscode.CompletionItemKind.Class));
  }
  for (const s of STDLIB) {
    const item = new vscode.CompletionItem(s.label, vscode.CompletionItemKind.Function);
    item.detail = s.detail;
    item.insertText = new vscode.SnippetString(s.insert);
    items.push(item);
  }

  // Identifiers already in file
  const seen = new Set();
  const re = /\b([A-Za-z_][A-Za-z0-9_]*)\b/g;
  let m;
  const text = document.getText();
  while ((m = re.exec(text))) {
    const name = m[1];
    if (seen.has(name) || KEYWORDS.includes(name) || TYPES.includes(name)) continue;
    seen.add(name);
    items.push(new vscode.CompletionItem(name, vscode.CompletionItemKind.Variable));
  }

  return items;
}

module.exports = { provideCompletions };
