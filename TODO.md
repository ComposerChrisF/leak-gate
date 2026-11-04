# TODO

The order of open work.

## Open

- [ ] **Publish this repository** as `ComposerChrisF/leak-gate`, public, and run its CI.  It waits on the maintainer.
- [ ] **Commit and tag messages:** `leak-gate message <file>` for a `commit-msg` hook, and `leak-gate range <A..B>` for CI over the commit and annotated tag messages a push adds.
- [ ] **A home-relative path rule** in the generic layer: `~/`, `$HOME/`, `${HOME}/`, or `env!("HOME")`, followed by a folder that is not a dot-folder.
- [ ] **A file-type rule:** `refuse-extensions` outside `allow-under`, set in `.leak-gate.toml`.
