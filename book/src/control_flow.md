# Control Flow

Rune supports a number of control flow expressions. We will be dedicating this
section to describe the most common ones.

## Conditions are `bool`

Conditions in `if`, `else if`, `while` and match guards must evaluate to a
`bool`. Both evaluated operands of `&&` and `||` must also be `bool`.
A different type is a runtime error at that expression; integers are not
truth values. `assert!(1)` is an error, while `assert!(true)` succeeds.
Assertion macro type errors currently point to the whole `assert!(...)` invocation.

Short-circuiting is preserved: `false && expression` and `true || expression`
do not evaluate or type-check the right operand. Integer `!` remains bitwise:
`!1` is `-2`. Pattern conditions (`if let` and `while let`) keep their usual
matching behavior.

This deliberately tightens Rune 0.14.2 behavior:

| Expression | Rune 0.14.2 | Current Rune |
| --- | --- | --- |
| `if 1 { 2 } else { 3 }` | `3` | type error at `1` |
| `true && 5` | `5` | type error at `5` |
| `false && 5` | `false` | `false` |
| `match 1 { _ if 1 => 0, _ => 2 }` | `0` | type error at guard `1` |
| `assert!(0)` | succeeds | type error |

Object Debug presentation also uses sorted key order, recursively. This is a
presentation rule; object iteration and serialization order are unchanged.

## `return` expression

In the previous section we talked about functions. And one of the primary things
a function does is return things. The `return` expression allows for returning
from the current function. If used without an argument, the function will return
a unit `()`.

The last statement in a function is known as an *implicit return*, and will be
what the function returns by default unless a `return` is specified.

```rune
{{#include ../../scripts/book/control_flow/numbers_game.rn}}
```

```text
$> cargo run -- run scripts/book/control_flow/numbers_game.rn
less than one
something else
```

## `if` expressions

If expressions allow you to provide a condition with one or more code branches.
If the condition is `true`, the provided block of code will run.

```rune
{{#include ../../scripts/book/control_flow/conditional.rn}}
```

```text
$> cargo run -- run scripts/book/control_flow/conditional.rn
The number is smaller than 5
```

Optionally, we can add another branch under `else`, which will execute in case
the condition is false.

```rune
{{#include ../../scripts/book/control_flow/conditional_else.rn}}
```

```text
$> cargo run -- run scripts/book/control_flow/conditional_else.rn
the number is smaller than 5
```

We can also add an arbitrary number of `else if` branches, which allow us to
specify many different conditions.

```rune
{{#include ../../scripts/book/control_flow/conditional_else_ifs.rn}}
```

```text
$> cargo run -- run scripts/book/control_flow/conditional_else_ifs.rn
the number is smaller than 5
```

Do note however that if you have *many* conditions, it might be cleaner to use
a `match`.

This will be covered in a later section, but here is a sneak peek:

```rune
{{#include ../../scripts/book/control_flow/first_match.rn}}
```

```text
$> cargo run -- run scripts/book/control_flow/first_match.rn
the number is smaller than 5
```
