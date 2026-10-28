# V2a error-class audit (WIP)

This is a source inventory for review, not an implemented projection. A row
marked evidence needed is not permission to collapse it or classify it as unreachable.
The public projections and their tests require a reviewed semantic table.

| Legacy VM kind | Proposed shared class | Evidence / reason |
| --- | --- | --- |
| `AllocError` | Allocation | A failed allocation projects to Allocation. |
| `AccessError` | AccessConflict | Existing live observer maps this legacy class; retain operation and typed detail separately. |
| `StackError` | Unclassified | Legacy construction/use: `runtime/memory.rs`; establish admitted reachability and fresh counterpart. |
| `SliceError` | Unclassified | Inspect legacy constructors and conversions in `runtime/vm_error.rs`; establish admitted reachability. |
| `BadInstruction` | Unclassified | Inspect legacy constructors and conversions in `runtime/vm_error.rs`; establish admitted reachability. |
| `BadJump` | Unclassified | Inspect legacy constructors and conversions in `runtime/vm_error.rs`; establish admitted reachability. |
| `DynArgsUsed` | Unclassified | Inspect legacy constructors and conversions in `runtime/vm_error.rs`; establish admitted reachability. |
| `Panic` | Panic | The live observers preserve panic class and message. |
| `NoRunningVm` | Unclassified | Legacy construction/use: `runtime/vm_execution.rs`; establish admitted reachability and fresh counterpart. |
| `Halted::Limited` | BudgetExhausted | Both engines report exhaustion; compare configured budget and point where comparable, never remaining units. |
| `Halted::Yielded` | Unclassified | Verify generator refusal before classifying as unreachable. |
| `Halted::Awaited` | Unclassified | Verify async refusal before classifying as unreachable. |
| `Overflow` | Overflow | Checked positive overflow. |
| `Underflow` | Overflow | Checked arithmetic underflow, retained as typed detail. |
| `DivideByZero` | DivisionByZero | Integer divisor is zero. |
| `MissingEntry` | Unclassified | Legacy construction/use: `runtime/vm.rs`; establish admitted reachability and fresh counterpart. |
| `MissingEntryHash` | Unclassified | Legacy construction/use: `runtime/vm.rs`; establish admitted reachability and fresh counterpart. |
| `MissingFunction` | Unclassified | Legacy construction/use: `compile/const_eval.rs`, `runtime/vm.rs`; establish admitted reachability and fresh counterpart. |
| `MissingContextFunction` | Unclassified | Legacy construction/use: `runtime/vm.rs`; establish admitted reachability and fresh counterpart. |
| `NotOwned` | Unclassified | Inspect legacy constructors and conversions in `runtime/vm_error.rs`; establish admitted reachability. |
| `MissingProtocolFunction` | TypeMismatch | Existing live observer maps this legacy class; retain operation and typed detail separately. |
| `MissingInstanceFunction` | TypeMismatch | Existing live observer maps this legacy class; retain operation and typed detail separately. |
| `IpOutOfBounds` | Unclassified | Legacy construction/use: `runtime/vm.rs`; establish admitted reachability and fresh counterpart. |
| `UnsupportedBinaryOperation` | TypeMismatch | Existing live observer maps this legacy class; retain operation and typed detail separately. |
| `UnsupportedUnaryOperation` | TypeMismatch | Existing live observer maps this legacy class; retain operation and typed detail separately. |
| `MissingStaticString` | Unclassified | Legacy construction/use: `runtime/vm.rs`; establish admitted reachability and fresh counterpart. |
| `MissingStaticBytes` | Unclassified | Legacy construction/use: `runtime/vm.rs`; establish admitted reachability and fresh counterpart. |
| `MissingStaticObjectKeys` | Unclassified | Legacy construction/use: `runtime/vm.rs`; establish admitted reachability and fresh counterpart. |
| `MissingDropSet` | Unclassified | Legacy construction/use: `runtime/vm.rs`; establish admitted reachability and fresh counterpart. |
| `MissingGlobals` | Unclassified | Legacy construction/use: `runtime/vm.rs`; establish admitted reachability and fresh counterpart. |
| `BadGlobalSlot` | Unclassified | Legacy construction/use: `runtime/vm.rs`; establish admitted reachability and fresh counterpart. |
| `UninitializedGlobal` | Unclassified | Legacy construction/use: `runtime/vm.rs`; establish admitted reachability and fresh counterpart. |
| `MissingRtti` | Unclassified | Legacy construction/use: `runtime/vm.rs`; establish admitted reachability and fresh counterpart. |
| `BadArgumentCount` | ArgumentCount | Call arity does not match. |
| `MaxValueDepth` | ValueDepth | Structural value walk exceeds its bound. |
| `MaxConstDepth` | Unclassified | Legacy construction/use: `runtime/const_value.rs`; establish admitted reachability and fresh counterpart. |
| `MaxConstSize` | Unclassified | Legacy construction/use: `runtime/const_value.rs`; establish admitted reachability and fresh counterpart. |
| `MalformedConstValue` | Unclassified | Legacy construction/use: `runtime/const_value.rs`; establish admitted reachability and fresh counterpart. |
| `MaxExecutionDepth` | Unclassified | Legacy nested protocol execution limit (`runtime/env.rs`); verify whether certified host paths can reach it. |
| `BadEnvironmentCount` | Unclassified | Legacy construction/use: `runtime/vm.rs`; establish admitted reachability and fresh counterpart. |
| `BadArgument` | Unclassified | Legacy construction/use: `experimental_core/execute/host_bridge.rs`, `function/mod.rs`; establish admitted reachability and fresh counterpart. |
| `UnsupportedIndexSet` | TypeMismatch | Existing live observer maps this legacy class; retain operation and typed detail separately. |
| `UnsupportedIndexGet` | TypeMismatch | Existing live observer maps this legacy class; retain operation and typed detail separately. |
| `UnsupportedTupleIndexGet` | TypeMismatch | Existing live observer maps this legacy class; retain operation and typed detail separately. |
| `UnsupportedTupleIndexSet` | TypeMismatch | Existing typed observer maps fresh TupleIndexSet detail to this legacy kind. |
| `UnsupportedObjectSlotIndexGet` | TypeMismatch | Existing live observer maps this legacy class; retain operation and typed detail separately. |
| `UnsupportedObjectSlotIndexSet` | Unclassified | Legacy construction/use: `runtime/vm.rs`; establish admitted reachability and fresh counterpart. |
| `UnsupportedIs` | Unclassified | Legacy construction/use: `runtime/vm.rs`; establish admitted reachability and fresh counterpart. |
| `UnsupportedAs` | InvalidCast | Live unit-to-integer cast produces fresh InvalidCast; preserve operand and target details. |
| `UnsupportedAsTarget` | InvalidCast | Live integer-to-bool cast produces fresh InvalidCast; preserve operand and target details. |
| `UnsupportedCallFn` | TypeMismatch | Live scalar call produces fresh Type; preserve callable type and site. |
| `MissingIndex` | IndexOutOfBounds | Existing live observer maps this legacy class; retain operation and typed detail separately. |
| `MissingIndexInteger` | IndexOutOfBounds | Existing live observer maps this legacy class; retain operation and typed detail separately. |
| `MissingIndexKey` | Unclassified | Legacy construction/use: `modules/collections/hash_map.rs`; establish admitted reachability and fresh counterpart. |
| `OutOfRange` | IndexOutOfBounds | Existing live observer maps this legacy class; retain operation and typed detail separately. |
| `UnsupportedTryOperand` | TypeMismatch | Existing typed observer maps fresh TryOperand detail to this legacy kind. |
| `UnsupportedIterRangeInclusive` | TypeMismatch | The existing typed observer maps invalid range endpoint types. |
| `UnsupportedIterRangeFrom` | TypeMismatch | The existing typed observer maps invalid range endpoint types. |
| `UnsupportedIterRange` | TypeMismatch | Existing live observer maps this legacy class; retain operation and typed detail separately. |
| `UnsupportedIterNextOperand` | Unclassified | Legacy construction/use: `runtime/vm.rs`; establish admitted reachability and fresh counterpart. |
| `ExpectedType` | TypeMismatch | Legacy expected-type errors can arise in script operations and host arguments; fresh Type and Host details both project to TypeMismatch. |
| `ExpectedAny` | Unclassified | Inspect legacy constructors and conversions in `runtime/vm_error.rs`; establish admitted reachability. |
| `ExpectedNumber` | TypeMismatch | Legacy expected-number errors can arise in script operations and host arguments; fresh Type and Host details both project to TypeMismatch. |
| `ExpectedEmpty` | Unclassified | Inspect legacy constructors and conversions in `runtime/vm_error.rs`; establish admitted reachability. |
| `ExpectedTuple` | Unclassified | Inspect legacy constructors and conversions in `runtime/vm_error.rs`; establish admitted reachability. |
| `ExpectedStruct` | Unclassified | Inspect legacy constructors and conversions in `runtime/vm_error.rs`; establish admitted reachability. |
| `MissingConstantConstructor` | Unclassified | Legacy construction/use: `runtime/vm.rs`; establish admitted reachability and fresh counterpart. |
| `ValueToIntegerCoercionError` | IntegerCoercion | Existing live observer maps this legacy class; retain operation and typed detail separately. |
| `IntegerToValueCoercionError` | Unclassified | Legacy construction/use: `runtime/value/macros.rs`; establish admitted reachability and fresh counterpart. |
| `ExpectedTupleLength` | TypeMismatch | Existing typed observer maps fresh tuple-length detail; retain actual and expected lengths. |
| `ExpectedVecLength` | Unclassified | Legacy construction/use: `runtime/vec_tuple.rs`, `runtime/vec.rs`; establish admitted reachability and fresh counterpart. |
| `ConstNotSupported` | Unclassified | Legacy construction/use: `runtime/const_value.rs`; establish admitted reachability and fresh counterpart. |
| `MissingInterfaceEnvironment` | Unclassified | Legacy construction/use: `runtime/env.rs`; establish admitted reachability and fresh counterpart. |
| `ExpectedExitedExecutionState` | Unclassified | Legacy construction/use: `runtime/vm_execution.rs`; establish admitted reachability and fresh counterpart. |
| `GeneratorComplete` | Unclassified | Legacy construction/use: `runtime/stream.rs`, `runtime/vm_execution.rs` and other sites; establish admitted reachability and fresh counterpart. |
| `FutureCompleted` | Unclassified | Legacy construction/use: `runtime/future.rs`; establish admitted reachability and fresh counterpart. |
| `MissingVariant` | Unclassified | Legacy construction/use: `lib.rs`; establish admitted reachability and fresh counterpart. |
| `MissingField` | IndexOutOfBounds | Existing live observer maps this legacy class; retain operation and typed detail separately. |
| `MissingVariantName` | Unclassified | Legacy construction/use: `lib.rs`; establish admitted reachability and fresh counterpart. |
| `MissingStructField` | Unclassified | Legacy construction/use: `lib.rs`; establish admitted reachability and fresh counterpart. |
| `MissingTupleIndex` | Unclassified | Legacy construction/use: `lib.rs`; establish admitted reachability and fresh counterpart. |
| `ExpectedVariant` | Unclassified | Legacy construction/use: `lib.rs`; establish admitted reachability and fresh counterpart. |
| `UnsupportedObjectFieldGet` | Unclassified | Legacy construction/use: `lib.rs`; establish admitted reachability and fresh counterpart. |
| `IllegalFloatComparison` | IllegalFloatComparison | Ordering a NaN is undefined. |
| `IllegalFloatOperation` | Unclassified | Legacy NaN hashing errors; `std::ops::hash(0.0 / 0.0)` is refused Native by fresh in a live test. Other reachable hashing paths still need review. |
| `MissingCallFrame` | Unclassified | Legacy construction/use: `runtime/vm_call.rs`; establish admitted reachability and fresh counterpart. |
| `IllegalFormat` | Unclassified | Legacy `format!("{:x}", true)` errors; fresh refuses FormatSpec in a live test. Other formatting paths still need review. |

## Fresh execution kinds

| Fresh kind | Proposed shared class | Evidence / reason |
| --- | --- | --- |
| `Unsupported` | Unclassified | Reachability and stock counterpart require review. |
| `Type` | TypeMismatch | Type detail must distinguish operand and operation. |
| `IllegalFloatComparison` | IllegalFloatComparison | Retains operand bits. |
| `Overflow` | Overflow | Retains underflow distinction as detail. |
| `DivisionByZero` | DivisionByZero | Integer divisor is zero. |
| `Panic` | Panic | Script panic. |
| `IndexOutOfBounds` | IndexOutOfBounds | Index or field is missing. |
| `InvalidShift` | EngineOnly | Reviewed execution or embedding boundary; do not cross-compare. |
| `InvalidCast` | InvalidCast | Both invalid cast operands and targets are checked in the live projection audit. |
| `IntegerCoercion` | Unclassified | Reachability and stock counterpart require review. |
| `FormatLimit` | EngineOnly | Byte, depth and node limits are indistinguishable and cannot be mapped specifically to ValueDepth. |
| `ValueDepth` | ValueDepth | Structural walk exceeded its bound. |
| `Output` | EngineOnly | Reviewed execution or embedding boundary; do not cross-compare. |
| `Allocation` | Allocation | A failed allocation projects to Allocation. |
| `StoreCapacity` | None | Explicit fresh-engine resource or embedding boundary, per review ruling; fail closed in cross-engine comparisons. |
| `ClosedStore` | None | Explicit fresh-engine resource or embedding boundary, per review ruling; fail closed in cross-engine comparisons. |
| `InvalidHandle` | None | Explicit fresh-engine resource or embedding boundary, per review ruling; fail closed in cross-engine comparisons. |
| `AccessConflict` | Unclassified | Reachability and stock counterpart require review. |
| `FuelExhausted` | BudgetExhausted | Legacy Halted::Limited is the typed counterpart; budget units differ, so retain configured budget and comparable context. |
| `InvalidCode` | EngineOnly | Reviewed execution or embedding boundary; do not cross-compare. |
| `FrameCapacity` | EngineOnly | Reviewed execution or embedding boundary; do not cross-compare. |
| `Cancelled` | None | Explicit fresh-engine resource or embedding boundary, per review ruling; fail closed in cross-engine comparisons. |
| `Host` | Depends on typed detail | The cold bridge preserves the projected native class; unknown causes remain Unclassified. |
| `ArgumentCount` | ArgumentCount | Call arity does not match. |

## Reviewed projection policy

The two projections return `Class`, `EngineOnly`, or `Unclassified`; comparisons
accept only equal `Class` values and also compare applicable origin, site and
typed detail. Unclassified rows remain fail-closed and may be classified later
with a witness. This supersedes the earlier proposal to infer unreachable rows.
The approved mappings above include Allocation for legacy AllocError and fresh
Allocation, and BudgetExhausted for the legacy Limited halt and fresh fuel
exhaustion. Fresh StoreCapacity, ClosedStore, InvalidHandle, Cancelled,
FrameCapacity, FormatLimit, Output, InvalidCode and InvalidShift are EngineOnly.
Fresh Unsupported is Unclassified. Host conversions retain the projected legacy
class on the cold error path; unknown host causes remain Unclassified. Format
limits combine bytes, depth and nodes and are not projected as ValueDepth.
