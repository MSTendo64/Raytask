# RayTask

**RayTask** — кроссплатформенный язык программирования с единым синтаксисом для веба, десктопа, mobile, embedded и системного кода.

## Возможности

- Лексер → парсер → AST → байткод VM
- Транспиляция в C (`--target native`)
- CLI: `build`, `run`, `test`, `new`, `check`, `doc`, команды пакетов
- Классы, структуры, интерфейсы, generics, `var` / `dyn`, свойства, async
- Память: GC (по умолчанию в VM), `stack` / `owned` / `unsafe`
- Стандартная библиотека `bstd.*` (ядро через natives)

## Установка

Нужен [Rust](https://rustup.rs/) 1.70+:

```bash
cargo install --path .
```

Или из корня репозитория:

```bash
cargo build --release
# бинарник: target/release/raytask (или raytask.exe на Windows)
```

## Быстрый старт

```bash
raytask new myapp
cd myapp
raytask run src/main.rt
```

Пример:

```
import bstd.io;

void Main() {
    print("Hello, RayTask!");
    var x = 40;
    print($"Answer: {x + 2}");
}
```

## CLI

```bash
raytask check main.rt
raytask build main.rt --no-typecheck
raytask run main.rt
raytask build main.rt                              # → .rtbc
raytask build main.rt --target native              # → .c (+ gcc/clang), async+GC runtime
raytask build main.rt --target app --platform current
raytask build main.rt --target wasm
raytask build main.rt --target web
raytask build main.rt --target mobile
raytask build main.rt --target embedded --no-gc
raytask build main.rt --target kernel
raytask build main.rt --target native-bin --platform windows
raytask build main.rt --target efi
raytask build main.rt --target raw
raytask link main.rtbc --platform linux -o app.elf
raytask install SomeLib
raytask search http
raytask publish .
raytask test
raytask new myproject
raytask doc
```

### Product targets

| `--target` | Результат |
|------------|-----------|
| `bytecode` | `.rtbc` |
| `native` | C с GC + cooperative async |
| `app` | один исполняемый файл (VM + bytecode) |
| `native-bin` | PE/ELF/Mach-O через NativeCodeGen + Linker |
| `efi` / `raw` | UEFI `.efi` или плоский `.bin` |
| `wasm` / `web` | WASM/HTML-бандл (+ bytecode для web) |
| `mobile` | scaffolds Android / iOS |
| `embedded` / `kernel` | freestanding C (+ атрибуты ISR / MMIO) |

Удалённый registry: `RAYTASK_REGISTRY_URL` (`index.json` + packages).

### Проверка типов

Перед сборкой и запуском выполняется статический анализ:

- примитивы, `var` / `dyn`, nullable `T?`, массивы, `ptr<T>`, generics
- функции, return, аргументы вызовов
- классы / структуры / интерфейсы, поля, свойства, методы, `new`
- наследование и совместимость override
- операторы, присваивания, управляющие конструкции
- `unsafe` для указателей

```bash
raytask check examples/hello.rt
raytask check examples/bad_types.rt
```

### Standalone app (`--target app`)

Собирает **один исполняемый файл**: stub runtime (VM) + встроенный `.rtbc`.

```bash
raytask build examples/hello.rt --target app --platform current
./examples/dist/hello
```

Рядом создаётся портативный Cargo-проект `dist/<name>_app/` для пересборки на другой машине.

## Примеры

```bash
cargo run -- run examples/hello.rt
cargo run -- run examples/point.rt
cargo run -- test examples/tests.rt
```

## Конвейер компилятора

```
.rt source
   ↓ Lexer
 tokens
   ↓ Parser
 AST
   ↓ Compiler
 bytecode Module
   ↓ VM
 execution

AST ──→ CCodegen ──→ .c ──→ gcc/clang / emcc / freestanding ──→ native|wasm|kernel
```

## Документация

- [Спецификация](docs/SPEC.md)
- [Руководство](docs/GUIDE.md)
- [Стандартная библиотека](../stdlib/README.md)
- English: [../README.md](../README.md)

## Статус

Реализовано (ядро спеки §§1–30):

- Компилятор, typechecker, VM, транспиляция в C, product targets
- import, OOP, LINQ-запросы, операторы, свойства, indexers, extension methods
- async/await (VM + native C), FFI, GC (VM + native), closures, monomorphization
- `project.rtp`, install / search / publish, локальный и удалённый registry
- Препроцессор `#if`, `raytask doc`, `match` для Result, `using` / `owned`

```bash
raytask new myapp && cd myapp && raytask run
raytask build src/main.rt --target web
cargo test --test product_targets --test spec_gaps
```

## Лицензия

MIT
