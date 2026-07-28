# Руководство пользователя RayTask

Практическое руководство по сборке и запуску программ на RayTask.

## Структура проекта

```bash
raytask new myapp
```

Создаётся:

```
myapp/
  project.rtp
  src/main.rt
  README.md
```

Запуск без указания файла (берётся entry из `project.rtp`):

```bash
cd myapp
raytask run
```

### `project.rtp`

```
project "myapp" {
    version = "0.1.0"
    entry = "src/main.rt"

    dependencies {
    }

    build {
        optimize = "speed"
        target = "bytecode"
        gc = true
    }
}
```

## Сборка и запуск

```bash
raytask check src/main.rt          # parse + typecheck
raytask run src/main.rt            # typecheck + VM
raytask build src/main.rt          # записать .rtbc
raytask build src/main.rt --target native
```

Управление GC:

```bash
raytask run --gc              # по умолчанию
raytask run --no-gc
raytask run --gc-stress       # collect на каждую аллокацию
```

## Основы языка

### Импорты и точка входа

```
import bstd.io;
import bstd.collections;

void Main() {
    print("hi");
}
```

### Типы и переменные

```
int n = 1;
var inferred = 2;
dyn anything = "ok";
string? maybe = null;
```

### Классы и структуры

```
export class Point {
    int x;
    int y;

    new(x: int, y: int) {
        this.x = x;
        this.y = y;
    }

    int Manhattan(Point other) {
        return Abs(this.x - other.x) + Abs(this.y - other.y);
    }
}
```

Видимость: члены по умолчанию приватные; для публичного API — `export`.

### Async

```
import bstd.async;

async void Work() {
    await Task.Delay(100);
    print("done");
}

void Main() {
    Work();
}
```

### Память

- Объекты в куче по умолчанию под GC в VM (и в native C при включённом GC).
- Локалы `owned` вызывают Dispose при выходе из области видимости.
- `using (...) { ... }` вызывает `Dispose`.
- `unsafe` включает работу с указателями (`ptr<T>`, `*`, `&`).

### Препроцессор

```
#if DEBUG
    print("debug");
#endif

#if WINDOWS
    // ...
#elif LINUX
    // ...
#endif
```

## Product targets

| Команда | Назначение |
|---------|------------|
| `--target bytecode` | модуль VM (`.rtbc`) |
| `--target native` | исходник C / бинарник хоста |
| `--target app` | один exe со встроенным runtime |
| `--target wasm` | scaffold WebAssembly |
| `--target web` | бандл для браузера |
| `--target mobile` | scaffolds Android + iOS |
| `--target embedded` | freestanding C для MCU |
| `--target kernel` | freestanding C в стиле ядра (без GC) |
| `--target native-bin` | NativeCodeGen + Linker (OS-бинарник из байткода) |
| `--target efi` | UEFI-приложение `.efi` |
| `--target raw` | плоский образ `.bin` |

### Нативные бинарники из байткода

```bash
raytask build main.rt --target native-bin --platform windows
raytask build main.rt --target native-bin --platform linux
raytask build main.rt --target native-bin --platform macos
raytask build main.rt --target efi
raytask build main.rt --target raw
raytask link main.rtbc --platform uefi -o main.efi
```

Конвейер: `.rt` → RTBC → **NativeCodeGen** (`ObjectFile`) → **Linker** (PE / ELF / Mach-O / EFI / raw).

Атрибуты для embedded / kernel:

```
[address: 0x40021000]
struct GpioRegisters {
    uint mode;
}

[interrupt: 0x80]
void SystemCallHandler() {
}

[export: "kmain"]
void KernelMain() {
}
```

## Пакеты

```bash
raytask install SomeLib
raytask uninstall SomeLib
raytask update
raytask search Some
raytask publish .
```

Локальные пути: `.raytask/packages/`, `registry/`, `RAYTASK_REGISTRY`.  
Удалённо: `RAYTASK_REGISTRY_URL`.

## Тесты и документация

```bash
raytask test                  # функции с [test]
raytask doc                   # markdown из /// в docs/api/
```

```
[test]
void TestAdd() {
    assertEq(1 + 1, 2);
}
```

## FFI

```
[DllImport: "mylib"]
[include: "mylib.h"]
int foreign_add(a: int, b: int);

[c: "
int add(int a, int b) { return a + b; }
"]
```

См. `examples/ffi_demo.rt`, `examples/ffi_embed.rt`.

## Дополнительно

- [SPEC.md](SPEC.md) — язык и справочник `bstd`
- [stdlib/README.md](../../stdlib/README.md) — таблица модулей
- [English guide](../../docs/GUIDE.md)
