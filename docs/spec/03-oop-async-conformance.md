# RayTask Spec Chapter 3: OOP, Async, and Conformance

## 1. Object Model

- User-defined `class` declarations support constructors, instance methods, and `static` methods.
- Interface implementation is checked semantically.
- Override compatibility must preserve parameter compatibility and return-type compatibility.

## 2. Async Runtime Surface

The current normative async surface includes:

- `Task.Delay`
- `Task.Run`
- `Task.WhenAll`
- `Task.WhenAny`
- `CancellationTokenSource`
- `CancellationToken`
- `TaskGroup`

## 3. Cancellation Semantics

- Cancellation is cooperative.
- A cancelled token is observable through `IsCancellationRequested`.
- `ThrowIfCancellationRequested()` converts the token state into an operation failure at an explicit check point.
- Group cancellation applies to tasks registered in that structured scope.

## 4. Structured Concurrency Direction

`TaskGroup` represents the current structured-scope primitive:

- spawned tasks are tracked by the group
- `WhenAll` aggregates all tracked tasks
- `WhenAny` completes when the first tracked task completes
- `Cancel` marks the scope as cancelled and propagates cancellation to tracked tasks

## 5. Spec-to-Test Mapping

The following tests currently serve as conformance anchors:

- async behavior: `tests/async.rs`
- chapter-oriented semantic regression coverage: `tests/spec_conformance.rs`
- static member dispatch, interface contracts, generic constraints, `Task.WhenAny`, cancellation token state, and `TaskGroup`: `tests/next_layer.rs`
- product/backend sanity for async-enabled targets: `tests/product_targets.rs`

Every future semantic addition should update this mapping with at least one positive and one negative test when failure behavior is part of the contract.
