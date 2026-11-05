# Plan: Read a Home Path That rustfmt Splits Across Lines

## Problem

The home-directory rule reads Rust’s `env!("HOME")` only when `, "/<folder>` follows it on the same line, as the header says and `env_home_re` does.  rustfmt writes any `concat!` too long for one line with one argument per line:

```rust
const FIXTURE: &str = concat!(
    env!("HOME"),
    "/<folder>/…"
);
```

So the form a formatted Rust repository actually holds is the one the rule cannot see.  Reported on 4-Nov-2026 by a repository preparing to go public: of 13 lines with paths into the owner’s own folder, which it was about to remove, `--generic-only` found the 4 written with `~/` and none of the 9 written as above.  The owner’s private list catches them on his machine, but CI never has that list, so in CI any repository passes them.

The gate does what its header says, so this is not a bug.  The header promises less than the rule exists for, which is to put the drive rule in CI, where the private list never runs.

## Proposed Change

Read `env!("HOME")` followed by a `,` at the end of a line as joined to the next line.  When the next line begins, after any whitespace, with `"/<folder>`, check the folder as if both were on one line, with the same passing rule: dot-folders, the standard folders, and placeholders pass.  The finding names the line that holds the folder, since that is the line to change.  The header and README say so.

Other splits stay unread, such as a comment line between the two lines, or the `/` in a string literal of its own (`env!("HOME"), "/", "<folder>"`).  Whether any of them is worth reading is decided when this lands.

## Implementation Notes

- The scan reads one line at a time (`git grep -n`), so no single pattern can join two lines.  Instead, a second pass: find the lines that end in `env!("HOME"),`, allowing whitespace as `env_home_re` does, in both locales as `grep_index` does.  For each, read the next line of that blob from the index, and match `^[[:space:]]*"/` followed by `$folder` against `passing_re`, as today.
- The same join applies to messages (`scan_text`), though a message rarely holds Rust.
- A false positive is unlikely: `env!("HOME"),` at the end of a line, followed by `"/<name>`, is a home path by construction.
- Tests, each failing on 0.6.0: the rustfmt layout is refused, and the finding names the folder’s line.  A dot-folder, a standard folder, and a placeholder in that layout pass.  A next line that is not a string literal passes, and the same-line cases are unchanged.

## Why Not a Workaround

The private list catches these lines only on the owner’s machine, and CI runs `--generic-only`, the layer this rule was added for.  Writing the path on one line by hand, or widening rustfmt’s `max_width`, hides the form from the formatter, not from the reader, and every repository would have to know to do it.
