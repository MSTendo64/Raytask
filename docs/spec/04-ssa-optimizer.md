# Mid-level SSA IR and Optimizer

## Placement

After monomorphization, bytecode-producing compiles follow:

```text
AST → (stack bytecode frontend) → SSA lift → opt pipeline → bytecode Module (.rtbc)
```

`Optimize::None` validates the lift and retains the frontend Module bit-for-bit.
`Optimize::Speed` / `Optimize::Size` run the pass manager and re-emit stack bytecode.

**Embedded / kernel** and **host AOT** (`native` / `native-bin`) share the SSA pipeline, then lower to C:

```text
AST → bytecode → SSA lift → opt (+ phi-elim) → SSA→C bodies
                 ↘ AST still emits types, `[address:]`, consts, runtime preamble
```

Host AOT links the C with TCC/gcc into a real PE/ELF — **no RTBC interpreter** in the binary.
`--target app` remains the stub + embedded `.rtbc` packaging path.
Host `transpile_c` (AST-only dump) is unchanged.

## IR

Values are SSA (`ValueId`); control flow uses basic blocks with `Br` / `CondBr` / `Return`.
Mutable locals start as `Alloca` + `Load`/`Store`; `mem2reg` promotes them.
Effectful ops (`Call`, `Print`, `Throw`, `Await`, stores, FFI) are never DCE’d and are not speculated across `Await` or try regions.

SSA→C emits `bbN:` labels and `goto` for CFG, `int64_t` temps, and direct calls to freestanding helpers (`MmioRead32`, `Spin`, …).

## Passes by optimize level

See `src/ssa/pass.rs` (`pipeline_for`).

## Tests

`tests/ssa_opt.rs`, `tests/ssa_c.rs`, and `src/ssa` unit tests cover fold/DCE/SCCP/inline heuristics, runtime parity under `--optimize speed`, and embedded/kernel C emit.
