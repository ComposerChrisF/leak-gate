# leak-gate

A pre-commit and CI check that refuses to publish a private reference.  It scans the index of a git repository (what is committed, or about to be) and exits 3 when a tracked file’s contents or path holds:

- an absolute home path (`/Users/<name>`, `/home/<name>`, `C:\Users\<name>`), or a path into the home directory’s `.claude` folder;
- a bug or plan ID that the repository’s history cannot resolve: one below its series’ floor (`bugs/FLOOR`, `plans/FLOOR`), or any ID of a series with no floor;
- a name on a private list, which is never committed.

The first two are the **generic** layer, which runs anywhere, CI included.  The third is the **private** layer, which runs only where the list is.

`scripts/leak-gate --help` is the contract: what is scanned, what is excluded, and the exit codes.

## Use

The gate is the shell script `scripts/leak-gate`; it needs `bash`, `git`, `grep`, and `perl`.  A repository runs it in two places:

- **A pre-commit hook**, from a copy on `PATH`:

  ```sh
  #!/bin/sh
  exec leak-gate
  ```

- **CI**, from this repository at a pinned commit, checked against its SHA-256 before it runs, so that a change here reaches a repository only when that repository moves its pin.

Before publishing anything, run `leak-gate --require-private`, which refuses to pass without the private list.

## A repository’s own list

A repository names its private list in a committed `.leak-gate.toml` at its root:

```toml
private-list = "myproject"
```

The gate then reads `~/.config/leak-gate/lists/myproject.txt`, one extended regular expression per line, matched as whole words, ignoring case.  The key is committed and the list never is, so every clone on the same machine finds the same list, and each repository is checked against its own names rather than another’s.  A list the repository names must exist and hold a pattern, or the gate stops with exit 4; CI, which has no list, runs `leak-gate --generic-only`.  `$LEAK_GATE_PATTERNS` overrides the named list, and a repository that names none, with no `.leak-gate.toml` or with one that sets no `private-list`, uses `~/.config/leak-gate/patterns.txt` when it is there.

## Status

The script and its tests moved here from `cli-contract`, where the gate began, and each repository now names its own private list.  Still to come: a check of commit and annotated tag messages, as a `commit-msg` hook and over a pushed range in CI; a generic rule for paths relative to the home directory; and a per-repository rule that refuses given file types outside a cleared folder.

## Tests

`cargo test` runs the gate against scratch repositories.  The package exists only for the tests and is never published.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
