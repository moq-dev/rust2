# rust2-pub

Audits every `pub` written in a workspace: which another crate names, which only its own crate reaches, which only tests reach, and which nothing the workspace runs reaches.
It then narrows every `pub` no other crate names to `pub(crate)`, all at once, and checks that the workspace still checks and that every name in its source resolves to what it did.
The source on disk never changes: rustc reads the narrowed tokens through its file loader, and the result is a patch.

```bash
nix run github:kixel-ai/rust2#pub -- --out audit                # in the workspace, from its own dev shell
just driver pub --out audit --manifest-path a/Cargo.toml --manifest-path b/Cargo.toml -- --workspace --all-targets
```

It writes into `--out`:

- `report.txt`: the findings, each a `pub` with its file, line, and column, what reaches it, and the first place another crate or its own names it.
- `report.json`: every `pub`, with the same evidence.
- `narrow.patch`: the narrowing that checked, relative to the directory the report names.

Cargo args after `--` select what to check, `--workspace --all-targets` by default.
Give several `--manifest-path`s to audit workspaces that share crates as one, as rust2's drivers share `driver/`.

## How

The binary is rustc for cargo, as the other drivers are.
After type checking each local crate (the workspaces' members and their path dependencies), it writes every definition, every name the source resolves, and what each definition's signature and body refer to, by rustc's `DefPathHash`, which a crate depending on another computes the same.
Joined across every compilation, the graph gives:

- **Reachability** from what runs with nothing naming it: `main`, the test harness, exported symbols, `const _`, and a proc-macro crate's macros.
  A trait impl is reached once its trait and self type are, and then needs every item it declares.
  An import is reached once what it supplies is, which makes nothing else live: a glob supplies only the names rustc resolved through it (its `glob_map`), and a list holds its paths' `pub`, so it is reached once one of theirs is.
  Production and test roots give unreached, test-only, and crate-only items.
- **Who needs a `pub`**: any reference from another crate, from live or dead code, since dead code must still compile.
  That includes the re-export a path goes through, the re-exports a glob supplied each name through, the types an expression has, and a tuple constructor's fields.
- **What crates or tools outside the audit may use**: what a published library exports, and all of an unpublished library the audit does not run.
  The audit runs what has a binary, and the unpublished workspace members that builds on; any other library, such as a translator's fixture, is used outside cargo's view, by tools that may read its source.
  What that API exposes of another crate stays public too: a re-exported or returned type with its fields, variants, and inherent items, a trait with its items, all of a crate it re-exports with `pub extern crate`, and every `pub` in the definition of a macro it exports, which goes wherever its callers expand it.

Narrowing compiles every proposal together, restoring each `pub` a rustc error, or a `private_interfaces` or `private_bounds` lint, points at, until the workspaces check.
It then requires every name to resolve as before: a narrowed method can compile and still change which method a call picks, such as a slice's `len` through `Deref`.
And it requires every item to print as it did, by rustc's HIR printer, once macros have expanded.
That printer leaves visibility out, so an item prints differently only where a macro read a narrowed `pub`: as `stringify!` does a `$v:vis`, as a derive or attribute macro may its whole item, or by position, as `column!` does.
The audit traces each such item back through the macro calls it came from, however nested, and restores the narrowed tokens they read: those on a call's line up to its end, and those of the item an attribute or derive was given.

## Limits

Only the selected targets, features, and host platform are checked: doctests, other features and platforms, and crates outside the audit are unseen, so an item only those use can narrow.
Narrowing an unreached item exposes it to `dead_code`, which the patch does not delete.
An import that only dead code uses still counts as reached when what it supplies is reached elsewhere: rustc records which names a glob supplied, but not from where.
