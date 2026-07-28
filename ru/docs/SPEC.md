# Спецификация языка RayTask

Краткий справочник по языку и стандартной библиотеке. См. также [руководство](GUIDE.md).

## Язык

| Элемент | Описание |
|---------|----------|
| Расширение файлов | `.rt` |
| Точка входа | `void Main()` |
| Импорты | `import bstd.io;` |
| Видимость | `export` для публичного API |
| Параметры | `name: type` |
| Типизация | `dyn` (динамика), `var` (вывод) |
| Память | GC / `stack` / `owned` / `unsafe` |
| Флаги GC | `raytask run --gc` (по умолчанию) / `--no-gc` / `--gc-stress` |
| Runtime GC | `Gc.Collect()`, `Gc.Stats()`, `gc()`, финализаторы `~new` |
| Closures | лямбды захватывают локалы |
| Generics | monomorphization на этапе компиляции (`Id__int`, `Box__string`) |

## Product targets (`--target`)

| Target | Результат |
|--------|-----------|
| `bytecode` | `.rtbc` для VM |
| `native` | C + gcc/clang; runtime с **async (`RtTask` / `await`) и GC** |
| `app` | standalone-исполняемый файл (stub + bytecode) |
| `wasm` | C + HTML/JS shell (+ emcc/clang при наличии) |
| `web` | web-бандл: wasm scaffold + `embedded.rtbc` / `rtbc.js` |
| `mobile` | scaffolds Android + iOS с bytecode |
| `embedded` | freestanding C + `link.ld` |
| `kernel` | freestanding, GC выключен, `[export:"kmain"]`, `[interrupt:]` |
| `native-bin` | NativeCodeGen + Linker → PE/ELF/Mach-O (`--platform windows\|linux\|macos`) |
| `efi` | UEFI PE32+ (`.efi`) с freestanding mini-interpreter |
| `raw` | плоский бинарник (`.text` + RTBC) |

Также: `raytask link program.rtbc --platform windows|linux|macos|uefi|raw -o out`.

Атрибуты: `[address: 0x…]` (MMIO), `[interrupt: N]`, `[no_gc]`, `[export:]`, `[target: wasm]`.

## Пакеты / registry

- Манифест: `project.rtp`
- Команды: `raytask install` / `uninstall` / `update` / `search` / `publish`
- Локально: `registry/`, `RAYTASK_REGISTRY`
- Удалённо: `RAYTASK_REGISTRY_URL` → `GET /index.json`, `GET`/`POST /packages/{name}/{version}`

## Стандартная библиотека (`bstd`)

| Пространство | Описание |
|--------------|----------|
| `bstd.io` | Консоль: `print`, `write`, `readLine`, `readKey` |
| `bstd.fs` | `File`, `Directory`, `FileInfo` |
| `bstd.net` | HTTP / TCP / UDP (sync в текущей VM) |
| `bstd.async` | `Task.Delay` / `Task.Run` |
| `bstd.string` | Методы строк, `string.Join`, `StringBuilder` |
| `bstd.regex` | `regex(pattern)` |
| `bstd.json` | `Json.Parse` / `Stringify` |
| `bstd.yml` | `Yaml.Parse` / `Serialize` |
| `bstd.collections` | `List`, `Dictionary`, `Set`, `Queue`, `Stack` |
| `bstd.math` | `Math`, `Random` |
| `bstd.time` | `DateTime` |
| `bstd.crypto` | `Hash` (SHA256 / SHA1 / MD5) |
| `bstd.unsafe` | `malloc` / `free` / `sizeof` |
| FFI attrs | `[DllImport:]` / `[link:]` / `[include:]` / `[c:]` / `[abi:]` / `[export:]` |
| `bstd.result` | `Ok` / `Error` |
| `bstd.test` | `assert` / `assertEq` |
| `bstd.logging` | `Logger` |

API-заглушки: `stdlib/bstd/*.rt`. Реализация: `src/stdlib/`.
