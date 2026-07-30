# RayTask Spec Chapter 2: Type System

## 1. Type Categories

RayTask source types are grouped into these categories:

- value types: primitive numerics, `bool`, `char`, and user `struct` types
- reference types: `class`, interface-typed values, arrays, strings, tasks, and most runtime objects
- dynamic type: `dyn`
- nullable types: `T?`
- pointer types: `ptr<T>`
- generic type parameters: `T`, `K`, `V`, etc.

## 2. Assignment Rules

- Exact type identity is assignable.
- Numeric widening is assignable where defined by the implementation numeric lattice.
- `null` is assignable to nullable/reference-like targets.
- `T` is assignable to `T?`.
- `T?` is not implicitly assignable to `T`.
- Dynamic values may bypass static checking but do not redefine the language's nominal type rules.

## 3. Named and Generic Types

- Named types are nominal.
- Generic instantiations must satisfy arity and declared constraints.
- A generic constraint may require nominal compatibility with a class/interface bound.
- A `new()` constraint requires an accessible parameterless construction shape according to the current implementation model.

## 4. Constraints

RayTask currently recognizes:

- nominal bounds such as `where T: IFoo`
- constructor bounds such as `where T: new()`

Constraint validation is part of semantic analysis and must reject invalid instantiations before runtime or backend lowering.

## 5. Variance Direction

Variance is a semantic concept in RayTask, but support is intentionally conservative until the type model is fully stabilized. Unless a type family is explicitly declared otherwise, generic positions should be treated as invariant at the semantic layer.

## 6. Static vs Instance Members

- Instance methods may only be invoked with an instance receiver.
- Static methods may only be invoked through the declaring type.
- A static call site must not receive an implicit `this`.

## 7. Interface Contracts

- A type that declares an interface base must provide all required interface members with compatible signatures.
- Missing members are semantic errors even if the program never instantiates the type.
