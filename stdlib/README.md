# RayTask Standard Library (bstd)

API stubs live in `stdlib/bstd/*.rt`. Runtime implementations are VM natives in `src/stdlib/`.

| Module | Status | Notes |
|--------|--------|-------|
| `bstd.io` | native | `print`, `write`, `readLine`, `readKey` |
| `bstd.fs` | native | `File`, `Directory`, `FileInfo` |
| `bstd.collections` | native | `List`, `Dictionary`, `Set`, `Queue`, `Stack` |
| `bstd.string` | native | string methods + `string.Join` + `StringBuilder` |
| `bstd.regex` | native | `regex(pattern)`, `FindAll` / `IsMatch` / `Replace` |
| `bstd.math` | native | `Math.*`, `Random` |
| `bstd.time` | native | `DateTime.Now` / `UtcNow`, `GetTime` |
| `bstd.json` | native | `Json.Parse` / `Stringify` / `Serialize` |
| `bstd.yml` | native | `Yaml.Parse` / `Serialize` |
| `bstd.crypto` | native | `Hash.Sha256` / `Sha1` / `Md5` |
| `bstd.net` | sync native | `Http.Get`/`Post`, `TcpClient`, `UdpSocket` |
| `bstd.web` | sync native | `HttpServer`, `Web`, `Template` for server-side apps |
| `bstd.sqlite` | sync native | `Sqlite.Open`, `SqliteConnection.Execute/Query` |
| `bstd.async` | partial | `Task.Delay` (thread sleep); no full event loop |
| `bstd.unsafe` | native | `malloc` / `free` / `sizeof` (arena) |
| `bstd.hal` | native / freestanding C | `MmioRead32` / `MmioWrite32` / `Spin` — board kits in `examples/boards/` |
| `bstd.bots` | stub | bot reply helpers (compose with `bstd.net`) |
| `bstd.game` | stub | ticks / key stubs for game loops |
| FFI | language | `[DllImport:]` / `[link:]` / `[include:]` / `[c:]` — see `examples/ffi_demo.rt` |
| `bstd.result` | native | `Ok` / `Error`, `Result.IsOk` / `Value` / `Error` |
| `bstd.test` | native | `assert`, `assertEq` |
| `bstd.logging` | native | `Logger` / static `Logger.Info` etc. |

### Global helpers (§24.2)

`ParseInt`, `ParseFloat`, `ToString`, `IsNull`, `IsNotNull`, `IsNumeric`, `IsAlpha`, `IsEmail`, `sleep`, `GetTime`, `GenerateGuid`, `RandomInt`.

Imports (`import bstd.io;`) are documentation / tooling markers; symbols are available as VM globals.
