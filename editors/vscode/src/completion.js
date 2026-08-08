const vscode = require("vscode");

const KEYWORDS = [
  "import", "namespace", "module", "export", "protected", "private",
  "class", "struct", "interface", "abstract", "virtual", "override",
  "new", "base", "this", "super", "return", "if", "else", "switch", "case",
  "default", "break", "continue", "for", "foreach", "while", "do", "in",
  "try", "catch", "finally", "throw", "using", "unsafe", "stack", "owned",
  "async", "await", "match", "var", "dyn", "const", "property", "get", "set",
  "where", "operator", "params", "is", "as", "typeof", "sizeof",
  "when", "offsetof", "nameof",
];

const TYPES = [
  "void", "bool", "byte", "sbyte", "short", "ushort", "int", "uint",
  "long", "ulong", "float", "double", "decimal", "char", "string", "ptr",
  "List", "Dictionary", "Set", "Queue", "Stack", "Task", "Result",
  "DateTime", "TimeSpan", "File", "Directory", "StringBuilder", "Logger", "Http",
  "Json", "Yaml", "Math", "Random", "Hash", "Gc", "GC", "Object",
  "Exception", "IDisposable",
];

const STDLIB = [
  { label: "print", detail: "bstd.io", insert: "print($0);" },
  { label: "write", detail: "bstd.io", insert: "write($0);" },
  { label: "readLine", detail: "bstd.io", insert: "readLine()" },
  { label: "assert", detail: "bstd.test", insert: "assert($0);" },
  { label: "assertEq", detail: "bstd.test", insert: "assertEq($1, $2);" },
  { label: "Task.Delay", detail: "bstd.async", insert: "Task.Delay($0)" },
  { label: "Task.Run", detail: "bstd.async", insert: "Task.Run($0)" },
  { label: "Task.WhenAll", detail: "bstd.async", insert: "Task.WhenAll($0)" },
  { label: "Task.WhenAny", detail: "bstd.async", insert: "Task.WhenAny($0)" },
  { label: "GC.Collect", detail: "runtime", insert: "GC.Collect()" },
  { label: "File.ReadText", detail: "bstd.fs", insert: "File.ReadText($0)" },
  { label: "File.WriteText", detail: "bstd.fs", insert: "File.WriteText($1, $2)" },
  { label: "Json.Parse", detail: "bstd.json", insert: "Json.Parse($0)" },
  { label: "Json.Stringify", detail: "bstd.json", insert: "Json.Stringify($0)" },
  { label: "DateTime.Now", detail: "bstd.time", insert: "DateTime.Now" },
  { label: "DateTime.UtcNow", detail: "bstd.time", insert: "DateTime.UtcNow" },
  { label: "DateTime.Parse", detail: "bstd.time", insert: "DateTime.Parse($0)" },
  { label: "TimeSpan.FromSeconds", detail: "bstd.time", insert: "TimeSpan.FromSeconds($0)" },
  { label: "TimeSpan.FromMinutes", detail: "bstd.time", insert: "TimeSpan.FromMinutes($0)" },
  { label: "TimeSpan.FromHours", detail: "bstd.time", insert: "TimeSpan.FromHours($0)" },
  { label: "TimeSpan.FromMilliseconds", detail: "bstd.time", insert: "TimeSpan.FromMilliseconds($0)" },
  { label: "TimeSpan.Zero", detail: "bstd.time", insert: "TimeSpan.Zero" },
  { label: "Ok", detail: "bstd.result", insert: "Ok($0)" },
  { label: "Error", detail: "bstd.result", insert: "Error($0)" },
  { label: "String.IsNullOrWhiteSpace", detail: "bstd.string", insert: "String.IsNullOrWhiteSpace($0)" },
  { label: "String.IsNullOrEmpty", detail: "bstd.string", insert: "String.IsNullOrEmpty($0)" },
  { label: "Convert.ToInt", detail: "bstd.convert", insert: "Convert.ToInt($0)" },
  { label: "Convert.ToString", detail: "bstd.convert", insert: "Convert.ToString($0)" },
  { label: "Convert.ToFloat", detail: "bstd.convert", insert: "Convert.ToFloat($0)" },
  { label: "Math.Sqrt", detail: "bstd.math", insert: "Math.Sqrt($0)" },
  { label: "Math.Abs", detail: "bstd.math", insert: "Math.Abs($0)" },
  { label: "Math.Pow", detail: "bstd.math", insert: "Math.Pow($1, $2)" },
  { label: "Math.Sin", detail: "bstd.math", insert: "Math.Sin($0)" },
  { label: "Math.Cos", detail: "bstd.math", insert: "Math.Cos($0)" },
  { label: "Math.Max", detail: "bstd.math", insert: "Math.Max($1, $2)" },
  { label: "Math.Min", detail: "bstd.math", insert: "Math.Min($1, $2)" },
];

const IMPORTS = [
  "bstd.io", "bstd.fs", "bstd.net", "bstd.async", "bstd.string",
  "bstd.regex", "bstd.json", "bstd.yml", "bstd.collections", "bstd.math",
  "bstd.time", "bstd.crypto", "bstd.unsafe", "bstd.result", "bstd.test",
  "bstd.logging", "bstd.convert", "bstd.web", "bstd.sqlite", "bstd.threads",
  "bstd.compress", "bstd.reflect",
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

  // After `import ` — suggest bstd modules
  if (/import\s+[\w.]*$/.test(prefix)) {
    for (const imp of IMPORTS) {
      const item = new vscode.CompletionItem(imp, vscode.CompletionItemKind.Module);
      item.insertText = imp;
      items.push(item);
    }
    return items;
  }

  // Keywords with snippet inserts for common patterns
  for (const kw of KEYWORDS) {
    const item = new vscode.CompletionItem(kw, vscode.CompletionItemKind.Keyword);
    // Add snippets for common keywords
    if (kw === "using") {
      item.insertText = new vscode.SnippetString("using (${1:var} ${2:obj} = ${3:expr}) {\n\t$0\n}");
    } else if (kw === "catch") {
      item.insertText = new vscode.SnippetString("catch (${1:e}: ${2:TypeError}) {\n\t$0\n}");
    } else if (kw === "switch") {
      item.insertText = new vscode.SnippetString("switch (${1:value}) {\n\tcase ${2:pattern}: { break; }\n\tdefault: { break; }\n}");
    }
    items.push(item);
  }

  // Types
  for (const t of TYPES) {
    items.push(new vscode.CompletionItem(t, vscode.CompletionItemKind.Class));
  }

  // Stdlib functions with snippets
  for (const s of STDLIB) {
    const item = new vscode.CompletionItem(s.label, vscode.CompletionItemKind.Function);
    item.detail = s.detail;
    item.insertText = new vscode.SnippetString(s.insert);
    items.push(item);
  }

  // Identifiers already in file
  const seen = new Set();
  const re = /\b([A-Za-z_][A-Za-z0-9_]*)\b/g;
  const text = document.getText();
  let m;
  while ((m = re.exec(text))) {
    const name = m[1];
    if (seen.has(name) || KEYWORDS.includes(name) || TYPES.includes(name)) continue;
    seen.add(name);
    items.push(new vscode.CompletionItem(name, vscode.CompletionItemKind.Variable));
  }

  return items;
}

module.exports = { provideCompletions };
