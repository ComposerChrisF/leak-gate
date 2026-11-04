# leak-gate

A pre-commit and CI check that refuses to publish a private reference.  It scans the index of a git repository (what is committed, or about to be), a commit message, or the commit and annotated tag messages in a range, and exits 3 when a tracked file’s contents or path, or a message, holds:

- an absolute home path (`/Users/<name>`, `/home/<name>`, `C:\Users\<name>`), a path into the home directory’s `.claude` folder, or a path from the home directory into a folder that is not a dot-folder (`~/<folder>`, `$HOME/<folder>`, or in Rust `env!("HOME")` followed by `"/<folder>`), since a dot-folder such as `~/.config` names tool state and any other folder the owner’s own files;
- a bug or plan ID that the repository’s history cannot resolve: one below its series’ floor (`bugs/FLOOR`, `plans/FLOOR`), or any ID of a series with no floor;
- a name on a private list, which is never committed;
- a tracked file of a kind the repository refuses, by the end of its name, outside the folders it allows.

The first two and the last are the **generic** layer, which runs anywhere, CI included.  The third is the **private** layer, which runs only where the list is.

`scripts/leak-gate --help` is the contract: what is scanned, what is excluded, and the exit codes.

## Use

The gate is the shell script `scripts/leak-gate`; it needs `bash`, `git`, `grep`, and `perl`.  A repository runs it in three places:

- **A pre-commit hook**, over the files, from a copy on `PATH`:

  ```sh
  #!/bin/sh
  exec leak-gate
  ```

- **A commit-msg hook**, over the message, which can still be fixed with `git commit --amend` before it is pushed:

  ```sh
  #!/bin/sh
  exec leak-gate message "$1"
  ```

  It reads every line of the file git hands it, comment lines included, because git keeps those for `git commit -m` and `-F` and the hook cannot tell which lines git will drop.  So a private name that git’s editor template shows (an untracked file’s name, or with `-v` a line being removed) refuses the commit too; commit with `-m` or `-F` instead.  A hook sees only commits made through it, so a clone sets `git config core.hooksPath .githooks` (or wherever the hooks are kept).  Commits made by cherry-pick, rebase, or `am`, and every tag message, reach no hook; CI and the scan before publishing read them.

- **CI**, with `--generic-only`, over the files and over the messages the event adds: `leak-gate --generic-only range <revisions>`, where `<revisions>` is `before..after` for a push; the whole history for the first push of the default branch; the default branch to the tip for a new branch; for a tag, `<tag>^!` and then the default branch to the tag; and the base to the head for a pull request.  This repository’s `.github/workflows/leak.yml` spells out each case.  It needs the whole history (`fetch-depth: 0`), since `range` refuses a shallow clone; it reads the messages even when the files fail; and it sits in a workflow of its own, with no run cancelled by a newer push.  Two pushes still get no run, and their messages are read by none: a push whose head commit’s message asks to skip CI (`[skip ci]` and the like), and a push of more than three tags at once, for which GitHub sends no event.  CI takes the gate from this repository at a pinned commit, checked against its SHA-256 before it runs, so that a change here reaches a repository only when that repository moves its pin.

Before publishing anything, run both `leak-gate --require-private`, over the files, and `leak-gate --require-private range --all`, over every commit and tag message in the history; each refuses to pass without the private list.

## A repository’s own list

A repository names its private list in a committed `.leak-gate.toml` at its root:

```toml
private-list = "myproject"
```

The gate then reads `~/.config/leak-gate/lists/myproject.txt`, one extended regular expression per line, matched as whole words, ignoring case.  The key is committed and the list never is, so every clone on the same machine finds the same list, and each repository is checked against its own names rather than another’s.  A list the repository names must exist and hold a pattern, or the gate stops with exit 4; CI, which has no list, runs `leak-gate --generic-only`.  `$LEAK_GATE_PATTERNS` overrides the named list, and a repository that names none, with no `.leak-gate.toml` or with one that sets no `private-list`, uses `~/.config/leak-gate/patterns.txt` when it is there.

## A repository’s refused kinds of file

A repository that must not publish some kind of file, such as music scores beside its own test fixtures, says so in `.leak-gate.toml`:

```toml
refuse-extensions = ["sib", "musx"]
allow-under = ["tests/fixtures"]
```

Every tracked file whose name ends in `.sib` or `.musx`, ignoring case, is refused, except below `tests/fixtures/` and in a path `.leak-gate-exclude` lists.  The rule is in the generic layer, so CI refuses it too.  Only the name is read: a file of that kind under another name is not seen.

## Status

The script and its tests moved here from `cli-contract`, where the gate began.  Each repository now names its own private list; the gate reads commit and annotated tag messages as well as files; it refuses a path from the home directory into the owner’s own folders; and each repository can refuse kinds of file outside the folders it clears.

## Tests

`cargo test` runs the gate against scratch repositories.  The package exists only for the tests and is never published.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
