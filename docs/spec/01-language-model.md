# RayTask Spec Chapter 1: Language Model

## 1. Scope

This chapter defines the baseline semantic vocabulary for RayTask source programs.

## 2. Source Form

- A RayTask source file uses the `.rt` extension.
- Top-level declarations may include imports, namespaces, modules, types, functions, constants, and attributes.
- `GUIDE.md` may describe surface syntax informally, but this chapter governs interpretation.

## 3. Managed Execution Model

RayTask is a managed language with explicit escape hatches.

- Ordinary objects, arrays, strings, tasks, and most user-defined classes are managed reference-like values.
- Value-like primitives (`int`, `bool`, `double`, etc.) behave as copyable values.
- `struct` declarations are the intended value-type surface.
- `unsafe`, pointer, freestanding, and native interop features may bypass managed guarantees, but only at explicit boundaries.

RayTask does not currently define or require a Rust-style borrow checker. Reference aliasing is legal unless a narrower rule is explicitly stated for a feature such as `unsafe` pointers, native interop, or future `ref`/`out`/`in` parameter categories.

## 4. Nullability

- `null` is the bottom value for nullable or reference-like positions.
- `T?` denotes an explicitly nullable form of `T`.
- Assigning `null` to non-null value-like positions is invalid.
- Null-safe member access (`?.`) preserves nullability in the result.

## 5. Dispatch Model

- Instance members are invoked on object instances and receive an implicit receiver.
- `static` members are invoked through the declaring type name and do not receive an implicit receiver.
- A source construct that relies on type-name member lookup is specified as a static access, not as an instance call with a magic receiver.

## 6. Generic Model

- Generic parameters participate in semantic checking.
- Constraints are part of program meaning and must be enforced before code generation.
- Monomorphization is an implementation strategy, not the definition of generic correctness.

## 7. Async Model

- Async is cooperative and task-based.
- `await` observes task completion at explicit suspension points.
- Cancellation is cooperative and should surface as a cancelled/faulted task outcome rather than unsafely tearing execution away from arbitrary code.

## 8. Compatibility Notes

Any change to nullability, dispatch, generic-constraint rules, or async task transitions must update this chapter and the conformance mapping chapter.
