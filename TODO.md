# TODO

The order of open work.

## Open

- [ ] **Publish this repository** as `ComposerChrisF/leak-gate`, public, and run its CI.  It waits on the maintainer.
- [ ] **A pre-push hook:** `leak-gate range` over what each pushed ref adds, both layers, so that commits no `commit-msg` hook sees (cherry-pick, rebase, `am`, `--no-verify`) and every tag message are read for private names before they leave the machine.
- [ ] **The file-type rule over history:** `range` reads only messages, so a refused file committed and later removed is published unseen.  Apply the rule to the names each commit in a range adds (`git diff-tree --name-only`).
- [ ] **More spellings of the home directory:** the home-directory rule reads `~`, `$HOME`, and `${HOME}`, but not `${HOME:?}` or `${HOME:-<default>}` (the form shellcheck asks for before `rm -rf`), nor `~<user>/`, which names a user as `/Users/<name>` does.  Decide whether each belongs in the generic layer.
- [ ] **One copy of the CI case logic:** each repository’s CI would copy the step that turns an event into a range.  A reusable workflow here, pinned by commit, would keep one copy.  Decide when the first repository pins the gate.
