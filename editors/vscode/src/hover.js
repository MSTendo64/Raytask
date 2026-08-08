const vscode = require("vscode");

const DOCS = {
  // --- I/O ---
  print: "Print values to stdout followed by a newline.\n\n`print(value1, value2, ...)`",
  write: "Write values to stdout without a trailing newline.",
  readLine: "Read a line from stdin. Returns `string?` (null on EOF).",

  // --- Program structure ---
  Main: "Program entry point. Must be `void Main()` or `async void Main()`.",
  import: "Import a module namespace.\n\nExample: `import bstd.io;`",
  namespace: "Declare a namespace scope for organizing types.",
  module: "Define a module with explicit exports.",

  // --- Variables & types ---
  var: "Local variable with type inference.\n\n`var x = 42;` — x is inferred as `int`.",
  dyn: "Dynamically typed value (escapes static checking at compile time).",
  const: "Compile-time constant. Must be initialized with a literal.",
  is: "Type-test operator: `obj is Type` returns `bool`.",
  as: "Safe cast operator: `obj as Type` returns `null` if cast fails.",

  // --- Visibility ---
  export: "Public visibility — makes types and members accessible from outside the module.",
  protected: "Visible in the declaring class and derived classes.",
  private: "Visible only within the declaring class.",

  // --- Classes & OOP ---
  class: "Reference-type class definition.",
  struct: "Value-type struct definition (copy semantics).",
  interface: "Contract definition. Classes implementing an interface must provide all its methods.",
  abstract: "Cannot be instantiated directly; may have abstract (unimplemented) members.",
  virtual: "Method can be overridden in derived classes.",
  override: "Replaces a virtual method from a base class.",
  new: "Constructor call (creates instance) or member hiding.",
  base: "Reference to the base class (used in constructors and overrides).",
  this: "Reference to the current instance.",
  super: "Alias for base; calls the base implementation.",

  // --- Properties ---
  property: "Define a property with optional `get` and `set` accessors.",
  get: "Property getter — returns the property value.",
  set: "Property setter — the incoming value is available as `value`.",

  // --- Control flow ---
  if: "Conditional branch. `if (cond) { ... } else if (...) { ... } else { ... }`",
  else: "Alternative branch for `if`.",
  switch: "Multi-way branch. Supports range patterns (`..`), multi-patterns (`|`), and guards (`when`).",
  case: "A match arm inside `switch`.",
  when: "Guard clause on a `case` pattern: `case x when x > 0 { ... }`",
  default: "Fallback arm when no `case` matches.",
  break: "Exit the innermost loop or switch block.",
  continue: "Skip to the next iteration of a loop.",
  for: "C-style for loop: `for (var i = 0; i < n; i++) { ... }`",
  foreach: "Iterate over a collection: `foreach (var item in list) { ... }`",
  while: "Pre-test loop: `while (cond) { ... }`",
  do: "Post-test loop: `do { ... } while (cond);`",
  return: "Return a value from a function (or exit void function).",

  // --- Exception handling ---
  try: "Begin a try-catch-finally block.\n\n`try { ... } catch (e: TypeError) { ... } finally { ... }`",
  catch: "Handle an exception. Can be typed: `catch (e: ExceptionType) { ... }` or bare: `catch { ... }`.\n\nThe exception value is a string; typed catches check if the string starts with the type name.",
  finally: "Cleanup block that always runs after try/catch.",
  throw: "Throw an exception. `throw \"error message\";`",

  // --- Resource management ---
  using: "Auto-dispose pattern.\n\n`using (var f = File.Open(path)) { ... }` — calls `Dispose()` at end of block.",
  owned: "Ownership hint: the value will be `Dispose()`d when leaving the scope.",

  // --- Async ---
  async: "Marks a function as asynchronous. Returns a `Task`.",
  await: "Suspend execution until a `Task` completes.",

  // --- Generics ---
  where: "Generic type constraint. `class Foo<T> where T : IComparable { ... }`",
  operator: "Define operator overload: `operator +(T a, T b): T { ... }`",
  params: "Variadic parameter: `void Fn(params int[] values) { ... }`",

  // --- Low-level / Systems ---
  unsafe: "Allow pointer operations (`ptr<T>`, `*`, `&`, `asm`).",
  stack: "Hint for stack allocation (MVP semantics).",
  typeof: "Get `Type` metadata for a type at compile time.",
  sizeof: "Get the byte size of a type at compile time.",
  offsetof: "Get the byte offset of a field within a struct.",

  // --- Pattern matching ---
  match: "Pattern match expression — supports `Result` Ok/Error arms.\n\n`match (result) { Ok(v) => { ... }, Error(e) => { ... } }`",

  // --- Stdlib types ---
  List: "Growable array from `bstd.collections`.\n\n`var lst = new List(); lst.Add(42);`",
  Dictionary: "Key-value map from `bstd.collections`.\n\n`var d = new Dictionary(); d[\"key\"] = value;`",
  Set: "Unique-element set from `bstd.collections`.",
  Queue: "FIFO queue from `bstd.collections`.",
  Stack: "LIFO stack from `bstd.collections`.",
  Task: "Async task handle.\n\n`Task.Delay(ms)`, `Task.Run(fn)`, `Task.WhenAll(...)`, `Task.WhenAny(...)`.",
  Result: "Discriminated union for Ok/Error returns from `bstd.result`.",
  DateTime: "Date/time value from `bstd.time`.\n\nStatic: `DateTime.Now`, `DateTime.UtcNow`, `DateTime.Parse(str)`.\nProperties: `Year`, `Month`, `Day`, `Hour`, `Minute`, `Second`, `Ticks`.\nMethods: `ToString()`, `Format(fmt)`.",
  TimeSpan: "Time duration from `bstd.time`.\n\nStatic: `TimeSpan.Zero`, `TimeSpan.FromSeconds(n)`, `TimeSpan.FromMinutes(n)`, `TimeSpan.FromHours(n)`, `TimeSpan.FromMilliseconds(n)`.\nProperties: `TotalMilliseconds`, `TotalSeconds`, `TotalMinutes`, `TotalHours`, `TotalDays`, `Milliseconds`, `Seconds`, `Minutes`, `Hours`, `Days`.",
  GC: "Garbage collector control.\n\n`GC.Collect()` — trigger full GC cycle.\n`GC.Stats()` — get GC statistics.",
  Gc: "Deprecated alias for `GC`. Use `GC` instead.",
  Json: "JSON parsing and stringify from `bstd.json`.",
  Yaml: "YAML parsing from `bstd.yml`.",
  Math: "Math functions from `bstd.math`.",
  File: "File I/O from `bstd.fs`.",
  String: "String utilities from `bstd.string`.",
  StringBuilder: "Efficient string builder from `bstd.string`.",
  Ok: "Construct a successful `Result` value. `Ok(value)`.",
  Error: "Construct a failed `Result` value. `Error(\"message\")`.",
  Exception: "Base exception type. All thrown strings are treated as exception messages.",
  IDisposable: "Interface for types with `Dispose()` method. Used by `using` statement.",
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
