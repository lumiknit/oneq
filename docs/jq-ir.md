# Resolved jq IR and compilation

The current engine compiles Pair syntax to the arena-based `Ir` in
`src/jq/ir.rs`, then emits VM code. Pair remains the source/formatter format;
optimization operates only on resolved IR.

```text
module loading + dependency order
    → lexical resolution / direct Pair lowering
    → closed function templates and inlining
    → unused function removal / arena compaction
    → literal folding / constant paths / arena compaction
    → VM emission
```

Effect analysis and calc extraction are still future passes. They belong after
inlining, where library wrappers no longer hide scalar expressions.

## Lowering and canonical forms

Names resolve to stable BindingId, FunctionId and LabelId values. Lexical name
snapshots are taken at declaration scopes, not at every expression. Function
parameters are filters; a value parameter is lowered to a filter invocation
followed by Bind. Bind preserves the original input for its body.

Lowering reads declaration continuations directly, without rebuilding Pair trees.
Assignments generate calls to the jq-defined `_assign` / `_modify` functions;
compound assignments bind the RHS before constructing the update filter.
Interpolated strings generate IR directly. No jq source is synthesized or parsed
at call sites. Module source import still copies/rebases Pairs at the source-map
boundary; recursive Pair normalization/validation and general lowering/emission
have not been replaced in this change.

The primitive registry contains native operations. The unchanged upstream
`builtin.jq` provides jq-defined functions. `compat.jq` supplies the additional
jq definitions previously stored as registry strings. Both are loaded once per
Session through the ordinary compiler.

`Ir::push` establishes these local invariants:

- Pipe and Concat flatten nested nodes of the same kind.
- Pipe omits Input; an empty Pipe becomes Input.
- A singleton Pipe or Concat becomes its child.
- No output has one representation: Concat([]). Primitive `empty` lowers to it
  only after lexical resolution, so a user-defined `empty` still works.
- An empty Path returns its base; an empty function Scope returns its body.

An empty stage does not erase earlier pipe stages: those can have observable
effects. Path nodes retain their original-input boundary for computed indices.
Array, Try, Alternative, folds and path tracking retain their execution boundaries.
In particular, `[A][]` is not rewritten to `A`: collection can change partial
output, error timing, cancellation and termination.

## Templates and substitution

Session owns a template store keyed by FunctionId. Name and arity lookup continues
to use Symbols, so aliases and shadowing need no second name table. A template
owns a compact reachable IR fragment and shares its immutable FileSet through Rc.

Definitions are processed in lexical dependency order. Already eligible callees
are expanded before the caller is considered for a template. A template may use
its own filter parameters, local bindings and local labels; it must have no free
binding/label/filter references or remaining function calls. This excludes
recursion and captured callees. Nested definitions that become unused are removed;
remaining nested closures conservatively prevent template creation.

Inlining substitutes each filter invocation with a separately copied argument
expression. It does not evaluate, cache or eagerly bind filter arguments.
Value-parameter Bind nodes remain in place. The copier gives declarations inside
each expansion fresh binding, label and function IDs, including nested functions
inside argument filters. Free references in an argument continue to refer to the
caller. Computed pattern keys, pattern alternatives and source spans are copied too.

`compiler/rewrite.rs` supplies common ID mapping and explicit-stack traversal.
`compiler/templates.rs` uses them for copying, eligibility, substitution and
reachability. Calls are reference edges, not structural traversal edges.

A template currently has a 512 node/edge cost ceiling; one compilation has an
expansion budget of 16,384. These bound optimization growth, not source program
size. A call remains an ordinary call when expansion is ineligible or too large.
Templates are fully expanded when stored; compilation needs no unbounded rewrite
fixpoint. Unused functions and unreachable arena nodes are removed before emission,
while REPL exports remain roots.

Only successfully compiled exports enter Session's store. Previously appended
templates become usable only after the corresponding runtime definition has
executed. Rollback restores names and initialized closures; stable IDs prevent a
discarded or redefined name from selecting the wrong template.

`CompileOptions::inline` defaults to true. Setting it to false skips template
expansion and its cleanup for that append; it retains lowering's canonical forms.
The embedded library uses the default compiler options when Session is created.

## Constant folding

`compiler/ir_const_fold.rs` runs after inlining, also when inlining is disabled.
It folds literal arithmetic and comparisons using the VM's scalar operations:
binary numeric arithmetic produces `Value::Float`, while unary negation preserves
decimal precision. Failed operations stay in IR so runtime error handling and
unreachable branches keep their semantics. Literal strings piped into trim,
ltrim, rtrim or ASCII case conversion are folded too. Expensive operations such
as string repetition and regex evaluation are not folded.

String index literals become interned keys. Fully constant string paths use
`ConstPath { base, steps: Vec<Symbol> }`, merging adjacent constant path nodes.
Mixed paths retain their original-input boundary and use `PathStep::Key` for
constant string steps; numeric indices, computed indices, iteration and slices
retain their existing semantics. The VM's `ConstPath` instruction directly looks
up interned object keys and only materializes strings for path tracking or errors.
Arena compaction removes obsolete operands and key literal nodes after folding.

Constant arrays and objects are also folded into literal values.

## Container plans and last

The emitter builds `ContainerPlan::Array` and `ContainerPlan::Object` layouts
containing inline constants and placeholders for evaluated values. `MakeContainer`
constructs any fixed number of elements, including one, in one instruction.
Fixed arrays require structurally
single-output expressions with no escaping declarations; other arrays retain
stream collection. Object layouts support streaming values and preserve their
Cartesian evaluation order; computed or duplicate keys use general object code.

For `lhs + <container expression>`, `ExtendContainer` evaluates RHS fields before
the LHS, matching infix ordering, then extends a uniquely owned container directly.
Shared values use copy-on-write. Incompatible types take the ordinary addition
error path. Constant RHS containers continue to use `InfixConst` and its owned
addition implementation. Up to eight dynamic fields use stack-local scratch.

Native `last(g)` lowers to `Last(g)`. Its VM region shares collection recovery
boundaries but retains only one value (`LastItem`/`EndLast`), using constant space.
It still executes the complete source, propagates errors, and emits nothing for
an empty source. The zero-argument `last` and user-defined overrides are unchanged.

## Verification

Bytecode emission assigns dense slots independently to the entry and each
function. IR declaration IDs remain stable for modules and REPL transactions;
they are not runtime vector indices. Locals (including their path), filter
parameters and function closures share the scope's slot space. Nested capture
requirements are propagated before emission fixes source/destination offsets.

Frames are immutable `Rc`-shared binding regions backed by vectors, without
HashMaps or copy-on-write. A call allocates its own self/parameter region over
the closure's captured environment. Each Bind/Define creates a fresh region,
so repeating a binding through a generator cannot overwrite an earlier capture.
Captures reference existing frame slots rather than copying their values.
Reads search the current region and its parents; captured slots redirect to
the original region. Return frames and choice points retain shared frame
references. Self-recursion is installed only in the call region, avoiding a
closure/environment ownership cycle. Session export maps remain keyed by IR IDs.

`tests/integration/compile.rs` compares inlining on/off for streams, errors,
bindings, labels, computed patterns, nested argument closures and path updates.
It also checks chained library/user inlining, retained recursive/captured calls,
REPL initialization/redefinition/rollback and bounded expansion.

The module, VM, CLI stack and jq oracle suites remain the broader compatibility
checks. Full effect/cardinality inference and calc execution are not implemented
by this compiler cleanup.
