# RayTask Language Specification

This document is the normative root of the RayTask language specification. `GUIDE.md` is tutorial and descriptive; this file and the chapter files under `docs/spec/` define source-level language behavior that the compiler, VM, native backend, and conformance tests are expected to follow.

## Normative Split

- `docs/GUIDE.md`: tutorial, onboarding, examples, workflows
- `docs/SPEC.md`: normative index, compatibility policy, chapter map
- `docs/spec/*.md`: normative semantic chapters
- `docs/REGISTRY_PROTOCOL.md`: sibling protocol specification, not part of the core language

## Language Model

RayTask follows a managed language model closer to C# than to Rust:

- reference/value semantics are explicit at the type-model level
- `null` is a valid value for reference-like and explicitly nullable types
- ownership and aliasing rules are enforced around managed references, `unsafe` pointers, and interop boundaries, not via a Rust-style borrow checker
- generic constraints and variance are semantic concepts, not purely codegen artifacts
- `static` members are part of the language model and are distinct from instance dispatch
- async is cooperative and task-based; cancellation is observable at async boundaries

## Compatibility Policy

RayTask source compatibility is governed by the following policy:

1. Changes that alter parsing, type checking, overload resolution, nullability, dispatch, async state transitions, or generic constraint behavior require a corresponding update in this spec.
2. Every normative chapter must map to one or more tests under `tests/`.
3. Behavior may evolve between milestones, but silent semantic drift between VM, native, and docs is considered a bug.
4. New syntax may be added in a backward-compatible way; changing the meaning of already-valid code requires an explicit spec update and regression coverage.

## Chapter Map

- `docs/spec/01-language-model.md`
- `docs/spec/02-type-system.md`
- `docs/spec/03-oop-async-conformance.md`

## Snapshot Reference

| Item | Current normative summary |
|------|---------------------------|
| File extension | `.rt` |
| Entry point | `void Main()` or `async void Main()` |
| Imports | `import bstd.io;` |
| Visibility | `export`, `protected`, `private` |
| Parameters | `name: type` |
| Dynamic typing escape hatch | `dyn` |
| Nullability surface | `T?` and reference-null support |
| Memory surface | managed GC + `stack` / `owned` / `unsafe` escape hatches |
| Async surface | `async`, `await`, `Task`, `TaskGroup`, cancellation tokens |
| Generic implementation | semantic checking + monomorphization |
| Dispatch | instance vs `static` members are distinct |

## Product Targets

| Target | Result |
|--------|--------|
| `bytecode` | `.rtbc` for the VM |
| `native` | generated C/native flow with RayTask runtime semantics |
| `app` | standalone executable (stub + bytecode) |
| `wasm` | C + HTML/JS shell |
| `web` | web bundle scaffold + embedded bytecode |
| `mobile` | Android + iOS scaffolds with bytecode |
| `embedded` | freestanding C + `link.ld` |
| `kernel` | freestanding, GC off, `[export:"kmain"]`, `[interrupt:]` |
| `native-bin` | NativeCodeGen + linker output |
| `efi` | UEFI PE32+ |
| `raw` | flat binary |

## Standard Library Surface

The normative language surface depends on library declarations in `stdlib/bstd/*.rt`. Runtime implementations live in `src/stdlib/`, `src/vm.rs`, and backend-specific lowering.
