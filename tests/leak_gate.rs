//! `scripts/leak-gate`, run against scratch repositories.
//!
//! The script's header is its contract: it scans the index (every tracked file's contents and
//! path, except the paths a tracked `.leak-gate-exclude` lists), a commit message, or the commit
//! and tag messages in a range, for home paths, private names, and bug or plan IDs below their
//! series' floor; exit 1 means "the gate itself failed", 3 means findings, and 4 setup needed.
//!
//! The leaking strings are assembled at run time, never spelled out here, so this file does not
//! itself trip the gate it tests.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A scratch home and a repository in it holding `files`, all staged.
struct Fixture {
    tmp: tempfile::TempDir,
}

impl Fixture {
    fn new(files: &[(&str, &[u8])]) -> Fixture {
        let fx = Fixture {
            tmp: tempfile::tempdir().unwrap(),
        };
        fs::create_dir(fx.home()).unwrap();
        fs::create_dir(fx.repo()).unwrap();
        fx.git(&["init", "-q"]);
        for (name, contents) in files {
            let path = fx.repo().join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, contents).unwrap();
            fx.git(&["add", name]);
        }
        fx
    }

    fn home(&self) -> PathBuf {
        self.tmp.path().join("home")
    }

    fn repo(&self) -> PathBuf {
        self.tmp.path().join("repo")
    }

    /// A command run in `dir` with the scratch home, and with git's other configuration and a
    /// calling hook's index shut out.
    fn command(&self, program: impl AsRef<std::ffi::OsStr>, dir: &Path) -> Command {
        let mut c = Command::new(program);
        c.current_dir(dir)
            .env("HOME", self.home())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("LEAK_GATE_PATTERNS")
            .env_remove("GIT_DIR")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_WORK_TREE");
        c
    }

    fn git(&self, args: &[&str]) {
        let out = self
            .command("git", &self.repo())
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "fixture: git {args:?} failed: {out:?}"
        );
    }

    /// git in the repository, with an identity and no signing, whatever its outcome.
    fn git_out(&self, args: &[&str]) -> Output {
        self.command("git", &self.repo())
            .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
            .args(["-c", "commit.gpgSign=false", "-c", "tag.gpgSign=false"])
            .args(args)
            .output()
            .unwrap()
    }

    /// Commits what is staged with `message`, and returns the commit's ID.
    fn commit(&self, message: &str) -> String {
        let out = self.git_out(&["commit", "-q", "--allow-empty", "-m", message]);
        assert!(out.status.success(), "fixture: commit: {}", show(&out));
        let out = self.git_out(&["rev-parse", "HEAD"]);
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }

    /// The gate run in `dir`, with `LEAK_GATE_PATTERNS` set to `patterns` when it is given.
    fn gate_in(&self, dir: &Path, patterns: Option<&Path>, args: &[&str]) -> Output {
        self.gate_env(dir, patterns, args, &[])
    }

    /// [`Fixture::gate_in`], with `env` set too.
    fn gate_env(
        &self,
        dir: &Path,
        patterns: Option<&Path>,
        args: &[&str],
        env: &[(&str, &str)],
    ) -> Output {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/leak-gate");
        let mut c = self.command("bash", dir);
        c.arg(script).args(args);
        if let Some(p) = patterns {
            c.env("LEAK_GATE_PATTERNS", p);
        }
        c.envs(env.iter().copied());
        c.output().unwrap()
    }

    /// Writes `text` to `rel` under the scratch home's `.config/leak-gate`.
    fn config_file(&self, rel: &str, text: &str) {
        let path = self.home().join(".config").join("leak-gate").join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    /// The gate over the repository, with an empty private list: the generic layer alone.
    fn generic_gate(&self) -> Output {
        let empty = self.tmp.path().join("empty-patterns.txt");
        fs::write(&empty, "").unwrap();
        self.gate_in(&self.repo(), Some(&empty), &[])
    }
}

fn show(out: &Output) -> String {
    format!(
        "exit {:?}; stdout: {}; stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).trim(),
        String::from_utf8_lossy(&out.stderr).trim()
    )
}

fn home_path() -> String {
    ["", "Users", "bob", "secret"].join("/")
}

/// `git grep -I` skipped every binary file.
#[test]
fn leak_gate_scans_a_tracked_binary_file() {
    let text = format!("x{}\n", home_path());
    let control = Fixture::new(&[("notes.txt", text.as_bytes())]).generic_gate();
    assert_eq!(
        control.status.code(),
        Some(3),
        "control: {}",
        show(&control)
    );

    let binary = format!("x\0{}\n", home_path());
    let out = Fixture::new(&[("blob.bin", binary.as_bytes())]).generic_gate();
    assert_eq!(
        out.status.code(),
        Some(3),
        "a staged binary file holding a home path was not scanned: {}",
        show(&out)
    );
}

/// A private list the gate cannot reach was read as absent, and the gate exited 0.
#[test]
fn leak_gate_unreachable_private_list_is_not_absent() {
    let fx = Fixture::new(&[("README.md", b"clean\n")]);

    // Control: no default list at all is absence, and the layer is skipped.
    let absent = fx.gate_in(&fx.repo(), None, &[]);
    assert_eq!(absent.status.code(), Some(0), "control: {}", show(&absent));

    // A dangling link where the default list goes.
    let dir = fx.home().join(".config").join("leak-gate");
    fs::create_dir_all(&dir).unwrap();
    std::os::unix::fs::symlink(dir.join("nowhere"), dir.join("patterns.txt")).unwrap();
    let dangling = fx.gate_in(&fx.repo(), None, &[]);
    assert_eq!(
        dangling.status.code(),
        Some(1),
        "a dangling link for the default list: {}",
        show(&dangling)
    );

    // SAFETY: geteuid reads a process attribute and cannot fail.
    if unsafe { libc::geteuid() } == 0 {
        eprintln!("SKIPPED the unsearchable case: running as root, where mode 000 proves nothing");
        return;
    }
    // A real list in a directory that cannot be searched.  The guard restores the mode even on a
    // panic, so the temporary directory can still be removed.
    struct Unlock<'a>(&'a Path);
    impl Drop for Unlock<'_> {
        fn drop(&mut self) {
            let _ = fs::set_permissions(self.0, fs::Permissions::from_mode(0o755));
        }
    }
    fs::remove_file(dir.join("patterns.txt")).unwrap();
    fs::write(dir.join("patterns.txt"), "secretname\n").unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o000)).unwrap();
    let unlock = Unlock(&dir);
    let locked = fx.gate_in(&fx.repo(), None, &[]);
    drop(unlock);
    assert_eq!(
        locked.status.code(),
        Some(1),
        "a default list in a directory that cannot be searched: {}",
        show(&locked)
    );
}

/// A `LEAK_GATE_PATTERNS` that names a missing file is the caller's assertion, and a
/// relative value is read from the caller's directory.
#[test]
fn an_explicit_leak_gate_patterns_that_is_missing_is_exit_1() {
    let fx = Fixture::new(&[("sub/a.txt", b"secretname\n")]);
    let missing = fx.tmp.path().join("no-such-patterns.txt");
    for args in [&[][..], &["--require-private"][..]] {
        let out = fx.gate_in(&fx.repo(), Some(&missing), args);
        assert_eq!(
            out.status.code(),
            Some(1),
            "LEAK_GATE_PATTERNS names a missing file, {args:?}: {}",
            show(&out)
        );
    }

    let sub = fx.repo().join("sub");
    fs::write(sub.join("pats.txt"), "secretname\n").unwrap();
    let out = fx.gate_in(&sub, Some(Path::new("pats.txt")), &[]);
    assert_eq!(
        out.status.code(),
        Some(3),
        "a relative LEAK_GATE_PATTERNS, run from the directory that holds it: {}",
        show(&out)
    );
}

/// A private name in a tracked file's path was never scanned, and git quotes a path that is not
/// ASCII unless told not to.
#[test]
fn leak_gate_refuses_a_private_name_in_a_tracked_file_path() {
    for name in ["secretname", "sécretname"] {
        let path = format!("examples/{name}.rs");
        let fx = Fixture::new(&[(path.as_str(), b"fn main() {}\n")]);
        let patterns = fx.tmp.path().join("patterns.txt");
        fs::write(&patterns, format!("{name}\n")).unwrap();
        let out = fx.gate_in(&fx.repo(), Some(&patterns), &["--require-private"]);
        assert_eq!(
            out.status.code(),
            Some(3),
            "a private name in a tracked path, {path:?}: {}",
            show(&out)
        );
    }
}

/// `grep -w` counts `_` as part of a word, so a private name joined to other text by `_`, as in a
/// Rust file name or identifier, was not matched.
#[test]
fn leak_gate_refuses_a_private_name_joined_by_an_underscore() {
    for (path, contents) in [
        ("examples/secretname_demo.rs", "fn main() {}\n"),
        ("src/lib.rs", "use secretname_core::Thing;\n"),
    ] {
        let fx = Fixture::new(&[(path, contents.as_bytes())]);
        let patterns = fx.tmp.path().join("patterns.txt");
        fs::write(&patterns, "secretname\n").unwrap();
        let out = fx.gate_in(&fx.repo(), Some(&patterns), &["--require-private"]);
        assert_eq!(
            out.status.code(),
            Some(3),
            "{path:?} holding {contents:?}: {}",
            show(&out)
        );
    }
}

/// Only the private names were matched against tracked paths, so a bug or plan ID in a path
/// passed.
#[test]
fn generic_layer_refuses_an_id_in_a_tracked_file_path() {
    let path = format!("tests/fixtures/{}.txt", ["bug", "0012"].join("-"));
    let out = Fixture::new(&[(path.as_str(), b"clean\n")]).generic_gate();
    assert_eq!(out.status.code(), Some(3), "{path:?}: {}", show(&out));
}

/// The generic patterns caught a home path only in its slash form, and the home `.claude` folder
/// only after a tilde.
#[test]
fn generic_layer_refuses_the_windows_and_home_variable_spellings() {
    let cases = [
        ["C:", "Users", "bob", "proj"].join("\\"),
        ["C:", "", "Users", "", "bob"].join("\\"),
        ["$HOME", ".claude", "rules"].join("/"),
        ["${HOME}", ".claude", "rules"].join("/"),
    ];
    let mut passed = Vec::new();
    for text in cases {
        let out = Fixture::new(&[("README.md", format!("{text}\n").as_bytes())]).generic_gate();
        if out.status.code() != Some(3) {
            passed.push(format!("{text:?}: {}", show(&out)));
        }
    }
    assert!(passed.is_empty(), "{passed:#?}");

    // The fixtures' `alice` stays allowed in both Windows spellings.
    for alice in [
        ["C:", "Users", "alice"].join("\\"),
        ["C:", "", "Users", "", "alice"].join("\\"),
    ] {
        let out = Fixture::new(&[("README.md", format!("{alice}\n").as_bytes())]).generic_gate();
        assert_eq!(out.status.code(), Some(0), "{alice:?}: {}", show(&out));
    }
}

/// The home-path rule took a user name only in ASCII letters, digits, `.`, `_`, and `-`.
#[test]
fn generic_layer_refuses_a_home_path_whose_user_name_is_not_ascii() {
    for text in [
        ["", "Users", "Ōla", "x"].join("/"),
        ["C:", "Users", "秘密", "x"].join("\\"),
    ] {
        let out = Fixture::new(&[("README.md", format!("{text}\n").as_bytes())]).generic_gate();
        assert_eq!(out.status.code(), Some(3), "{text:?}: {}", show(&out));
    }
}

/// An ID of `series` (`bug` or `plan`) with the four digits `n`.
fn id(series: &str, n: u32) -> String {
    format!("{series}-{n:04}")
}

/// The gate over `files` beside a bug floor of 40 and a plan floor of 10, as the public repository
/// carries them, with `config` set in the repository first.
fn gate_with_floors(files: &[(&str, &[u8])], config: &[&str]) -> Output {
    let bug_floor = format!(
        "# IDs below {0} were issued elsewhere.\n{0}\n",
        id("bug", 40)
    );
    let plan_floor = format!("{}\n", id("plan", 10));
    let mut all: Vec<(&str, &[u8])> = vec![
        ("bugs/FLOOR", bug_floor.as_bytes()),
        ("plans/FLOOR", plan_floor.as_bytes()),
    ];
    all.extend_from_slice(files);
    let fx = Fixture::new(&all);
    if !config.is_empty() {
        fx.git(config);
    }
    fx.generic_gate()
}

/// Only the paths a tracked `.leak-gate-exclude` lists go unscanned, each taken literally; without
/// it, or with it untracked, every tracked file is scanned, `TODO.md` included.
#[test]
fn only_a_tracked_exclude_file_narrows_the_scope() {
    let todo = format!("see {}\n", home_path());
    let out = Fixture::new(&[("TODO.md", todo.as_bytes())]).generic_gate();
    assert_eq!(
        out.status.code(),
        Some(3),
        "no exclude file: {}",
        show(&out)
    );

    let excluded = Fixture::new(&[
        ("TODO.md", todo.as_bytes()),
        (".leak-gate-exclude", b"# never published\nTODO.md\n"),
    ]);
    let out = excluded.generic_gate();
    assert_eq!(
        out.status.code(),
        Some(0),
        "TODO.md excluded: {}",
        show(&out)
    );

    let untracked = Fixture::new(&[("TODO.md", todo.as_bytes())]);
    fs::write(untracked.repo().join(".leak-gate-exclude"), "TODO.md\n").unwrap();
    let out = untracked.generic_gate();
    assert_eq!(
        out.status.code(),
        Some(3),
        "an untracked exclude file: {}",
        show(&out)
    );

    let glob = Fixture::new(&[("TODO.md", todo.as_bytes()), (".leak-gate-exclude", b"*\n")]);
    let out = glob.generic_gate();
    assert_eq!(
        out.status.code(),
        Some(3),
        "a `*` line is a name: {}",
        show(&out)
    );
}

/// An exclude file that leaves nothing to scan is never what was meant.
#[test]
fn an_exclude_file_that_leaves_nothing_to_scan_is_exit_1() {
    let out =
        Fixture::new(&[("README.md", b"clean\n"), (".leak-gate-exclude", b".\n")]).generic_gate();
    assert_eq!(out.status.code(), Some(1), "{}", show(&out));
}

/// A history cut from its predecessor's resolves four-digit IDs from its series' floor up, and
/// none of a series with no floor, wherever the ID sits: after a letter that is not ASCII, joined
/// by `_`, in a binary file, in a path, or under forced colors.
#[test]
fn an_id_is_refused_below_its_series_floor() {
    let (low, high) = (id("bug", 39), id("bug", 40));
    let short = ["bug", "050"].join("-");
    let texts = [
        (format!("{high} and {}", id("bug", 41)), Some(0)),
        (low.clone(), Some(3)),
        (id("plan", 10), Some(0)),
        (id("plan", 9), Some(3)),
        (short, Some(3)),
        (format!("修复了{low}"), Some(3)),
        (format!("fixture_{low}_input"), Some(3)),
    ];
    for (text, want) in texts {
        let out = gate_with_floors(&[("README.md", format!("{text}\n").as_bytes())], &[]);
        assert_eq!(out.status.code(), want, "{text:?}: {}", show(&out));
    }

    let paths = [
        (format!("bugs/{high}-x.md"), Some(0)),
        (format!("docs/修复{low}.md"), Some(3)),
        (format!("tests/x_{low}_in.txt"), Some(3)),
    ];
    for (path, want) in paths {
        let out = gate_with_floors(&[(path.as_str(), b"clean\n")], &[]);
        assert_eq!(out.status.code(), want, "{path:?}: {}", show(&out));
    }

    for (n, want) in [(&low, Some(3)), (&high, Some(0))] {
        let blob = format!("x\0see {n}\n");
        let out = gate_with_floors(&[("blob.bin", blob.as_bytes())], &[]);
        assert_eq!(
            out.status.code(),
            want,
            "a binary file citing {n}: {}",
            show(&out)
        );
    }

    for (n, want) in [(&low, Some(3)), (&high, Some(0))] {
        let text = format!("{n}\n");
        let out = gate_with_floors(
            &[("README.md", text.as_bytes())],
            &["config", "color.ui", "always"],
        );
        assert_eq!(
            out.status.code(),
            want,
            "{n} under forced colors: {}",
            show(&out)
        );
    }

    let bug_floor_only = Fixture::new(&[
        ("bugs/FLOOR", format!("{high}\n").as_bytes()),
        ("README.md", format!("{}\n", id("plan", 99)).as_bytes()),
    ])
    .generic_gate();
    assert_eq!(
        bug_floor_only.status.code(),
        Some(3),
        "no plan floor: {}",
        show(&bug_floor_only)
    );
}

/// A tracked floor the gate cannot read as one ID above 0000, in a regular file, is the gate
/// failing, never a series with no floor or a low one.
#[test]
fn a_floor_not_in_its_form_is_exit_1() {
    let floors = [
        ["bug", "40"].join("-"),
        format!("{}\n{}\n", id("bug", 40), id("bug", 41)),
        id("bug", 0),
        String::new(),
        "\n\n".to_string(),
        format!("{}\nstray\n", id("bug", 40)),
        format!("{}\r\n", id("bug", 40)),
    ];
    for floor in floors {
        let readme = format!("{}\n", id("bug", 39));
        let out = Fixture::new(&[
            ("bugs/FLOOR", floor.as_bytes()),
            ("README.md", readme.as_bytes()),
        ])
        .generic_gate();
        assert_eq!(out.status.code(), Some(1), "{floor:?}: {}", show(&out));
    }

    let fx = Fixture::new(&[("README.md", b"clean\n")]);
    fs::create_dir(fx.repo().join("bugs")).unwrap();
    std::os::unix::fs::symlink(id("bug", 40), fx.repo().join("bugs").join("FLOOR")).unwrap();
    fx.git(&["add", "bugs/FLOOR"]);
    let out = fx.generic_gate();
    assert_eq!(
        out.status.code(),
        Some(1),
        "a symlinked floor: {}",
        show(&out)
    );
}

/// The header is the gate's help, and `--help` prints it.
#[test]
fn leak_gate_help_prints_its_header() {
    let fx = Fixture::new(&[("README.md", b"clean\n")]);
    let out = fx.gate_in(&fx.repo(), None, &["--help"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{}", show(&out));
    assert!(text.contains("Exit: 0 clean;"), "{text}");
}

/// A list the repository names in `.leak-gate.toml` is read in place of the default list, even
/// with CRLF line endings; the settings are read from the index; and only a tracked
/// `.leak-gate.toml` names a list.
#[test]
fn a_named_private_list_replaces_the_default() {
    let settings = "# The private list.\n\nprivate-list = \"proj.x-1\"  # this repository's\n";
    let fx = Fixture::new(&[
        (".leak-gate.toml", settings.as_bytes()),
        ("a.txt", b"namedname\n"),
        ("b.txt", b"defaultname\n"),
    ]);
    fx.config_file("lists/proj.x-1.txt", "namedname\n");
    fx.config_file("patterns.txt", "defaultname\n");
    let out = fx.gate_in(&fx.repo(), None, &[]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(3), "{}", show(&out));
    assert!(
        stdout.contains("a.txt") && !stdout.contains("b.txt"),
        "{}",
        show(&out)
    );

    fx.config_file("lists/proj.x-1.txt", "namedname\r\n");
    let out = fx.gate_in(&fx.repo(), None, &[]);
    assert_eq!(out.status.code(), Some(3), "a CRLF list: {}", show(&out));

    // An unstaged edit is not what the commit publishes.
    fs::write(fx.repo().join(".leak-gate.toml"), "# names no list\n").unwrap();
    let out = fx.gate_in(&fx.repo(), None, &[]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.code() == Some(3) && stdout.contains("a.txt") && !stdout.contains("b.txt"),
        "an unstaged edit to .leak-gate.toml: {}",
        show(&out)
    );

    let untracked = Fixture::new(&[("b.txt", b"defaultname\n")]);
    fs::write(untracked.repo().join(".leak-gate.toml"), settings).unwrap();
    untracked.config_file("patterns.txt", "defaultname\n");
    let out = untracked.gate_in(&untracked.repo(), None, &[]);
    assert_eq!(
        out.status.code(),
        Some(3),
        "an untracked .leak-gate.toml: {}",
        show(&out)
    );
}

/// A list the repository names must exist and hold a pattern, with or without
/// `--require-private`; the default list is never read in its place.
#[test]
fn a_named_private_list_that_is_absent_or_empty_is_exit_4() {
    let fx = Fixture::new(&[
        (".leak-gate.toml", b"private-list = \"proj\"\n"),
        ("a.txt", b"clean\n"),
    ]);
    fx.config_file("patterns.txt", "defaultname\n");
    for args in [&[][..], &["--require-private"][..]] {
        let out = fx.gate_in(&fx.repo(), None, args);
        assert_eq!(
            out.status.code(),
            Some(4),
            "absent, {args:?}: {}",
            show(&out)
        );
    }
    for text in ["", "# only a comment\n\n"] {
        fx.config_file("lists/proj.txt", text);
        let out = fx.gate_in(&fx.repo(), None, &[]);
        assert_eq!(out.status.code(), Some(4), "{text:?}: {}", show(&out));
    }
}

/// `--generic-only` runs the generic layer alone, whatever names a list, and says so.
#[test]
fn generic_only_skips_the_private_layer() {
    let fx = Fixture::new(&[
        (".leak-gate.toml", b"private-list = \"proj\"\n"),
        ("a.txt", b"namedname\n"),
    ]);
    let out = fx.gate_in(&fx.repo(), None, &["--generic-only"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "a named list absent: {}",
        show(&out)
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("--generic-only:"),
        "{}",
        show(&out)
    );

    let patterns = fx.tmp.path().join("patterns.txt");
    fs::write(&patterns, "namedname\n").unwrap();
    let out = fx.gate_in(&fx.repo(), Some(&patterns), &["--generic-only"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "LEAK_GATE_PATTERNS set: {}",
        show(&out)
    );

    let leak = format!("{}\n", home_path());
    let fx = Fixture::new(&[("a.txt", leak.as_bytes())]);
    let out = fx.gate_in(&fx.repo(), None, &["--generic-only"]);
    assert_eq!(out.status.code(), Some(3), "a home path: {}", show(&out));

    let out = fx.gate_in(&fx.repo(), None, &["--generic-only", "--require-private"]);
    assert_eq!(out.status.code(), Some(2), "both flags: {}", show(&out));
}

/// `LEAK_GATE_PATTERNS` still overrides a list the repository names.
#[test]
fn leak_gate_patterns_overrides_a_named_list() {
    let fx = Fixture::new(&[
        (".leak-gate.toml", b"private-list = \"proj\"\n"),
        ("a.txt", b"envname\n"),
    ]);
    let patterns = fx.tmp.path().join("patterns.txt");
    fs::write(&patterns, "envname\n").unwrap();
    let out = fx.gate_in(&fx.repo(), Some(&patterns), &[]);
    assert_eq!(out.status.code(), Some(3), "{}", show(&out));
}

/// A `.leak-gate.toml` the gate cannot read in the header's form stops it, under `--generic-only`
/// too: a setting it could not read must never pass as no setting.
#[test]
fn a_leak_gate_toml_not_in_its_form_is_exit_1() {
    let texts = [
        "private-list = proj\n",
        "private-list = 'proj'\n",
        "private_list = \"proj\"\n",
        "private-list = \"proj\"\nprivate-list = \"other\"\n",
        "private-list = \"../proj\"\n",
        "private-list = \".proj\"\n",
        "private-list = \"\"\n",
        "private-list = \"a#b\"\n",
        "[gate]\nprivate-list = \"proj\"\n",
        "\0\n",
        "private-list = \"pr\0oj\"\n",
    ];
    for text in texts {
        let fx = Fixture::new(&[(".leak-gate.toml", text.as_bytes()), ("a.txt", b"clean\n")]);
        fx.config_file("lists/proj.txt", "namedname\n");
        for args in [&[][..], &["--generic-only"][..]] {
            let out = fx.gate_in(&fx.repo(), None, args);
            assert_eq!(
                out.status.code(),
                Some(1),
                "{text:?}, {args:?}: {}",
                show(&out)
            );
        }
    }

    // A symlink's blob is its target's name, so the name is itself a valid setting here: only the
    // regular-file check refuses it.
    let fx = Fixture::new(&[("a.txt", b"clean\n")]);
    std::os::unix::fs::symlink("private-list = \"proj\"", fx.repo().join(".leak-gate.toml"))
        .unwrap();
    fx.git(&["add", ".leak-gate.toml"]);
    fx.config_file("lists/proj.txt", "namedname\n");
    for args in [&[][..], &["--generic-only"][..]] {
        let out = fx.gate_in(&fx.repo(), None, args);
        assert_eq!(
            out.status.code(),
            Some(1),
            "a symlink, {args:?}: {}",
            show(&out)
        );
    }
}

/// The list `secretname`, written for `fx`, and its path.
fn secret_list(fx: &Fixture) -> PathBuf {
    let patterns = fx.tmp.path().join("patterns.txt");
    fs::write(&patterns, "secretname\n").unwrap();
    patterns
}

/// `message` refuses in a commit message what the index scan refuses in a file: both layers, with
/// IDs judged against the floors in the index.
#[test]
fn message_refuses_what_the_index_scan_refuses() {
    let fx = Fixture::new(&[("bugs/FLOOR", format!("{}\n", id("bug", 40)).as_bytes())]);
    let patterns = secret_list(&fx);
    let msg = fx.tmp.path().join("msg");
    let cases = [
        ("Fix the parser\n\nNothing private.\n".to_string(), Some(0)),
        (format!("Fix {}\n", id("bug", 41)), Some(0)),
        (format!("Fix {}\n", id("bug", 39)), Some(3)),
        (format!("Fix {}\n", id("plan", 41)), Some(3)),
        (format!("Fix it\n\nSee {}\n", home_path()), Some(3)),
        (
            format!("See {}\n", ["", "Users", "alice", "x"].join("/")),
            Some(0),
        ),
        (
            format!("See {}\n", ["~", ".claude", "rules"].join("/")),
            Some(3),
        ),
        (format!("修复{}\n", id("bug", 39)), Some(3)),
        ("Fix x\0secretname\n".to_string(), Some(3)),
        ("Fix the secretname import\n".to_string(), Some(3)),
    ];
    for (text, want) in cases {
        fs::write(&msg, &text).unwrap();
        let out = fx.gate_in(
            &fx.repo(),
            Some(&patterns),
            &["message", msg.to_str().unwrap()],
        );
        assert_eq!(out.status.code(), want, "{text:?}: {}", show(&out));
    }
    let out = fx.gate_in(
        &fx.repo(),
        Some(&patterns),
        &["message", msg.to_str().unwrap()],
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("message:1:"),
        "a finding names where it is: {}",
        show(&out)
    );

    let out = fx.gate_in(
        &fx.repo(),
        Some(&patterns),
        &["--generic-only", "message", msg.to_str().unwrap()],
    );
    assert_eq!(out.status.code(), Some(0), "--generic-only: {}", show(&out));
}

/// git hands a `commit-msg` hook the message before its cleanup, and keeps comment lines for `-m`
/// and `-F`, so every line is read: comment lines, the lines below a scissors line, and a line
/// holding a byte that is not UTF-8, which GNU grep would otherwise pass over in a UTF-8 locale.
#[test]
fn message_reads_every_line() {
    let fx = Fixture::new(&[("README.md", b"clean\n")]);
    let patterns = secret_list(&fx);
    let msg = fx.tmp.path().join("msg");
    let scissors = "# ------------------------ >8 ------------------------";
    let texts: [Vec<u8>; 5] = [
        b"Subject\n\n# secretname is the client\n".to_vec(),
        format!("Subject\n\n{scissors}\nsecretname below the cut\n").into_bytes(),
        b"Subject\n\n#\tnew file:   secretname.txt\n".to_vec(),
        b"Subject\n\nFix \xff secretname\n".to_vec(),
        [
            b"See \xff ".to_vec(),
            ["~", ".claude", "x"].join("/").into_bytes(),
            b"\n".to_vec(),
        ]
        .concat(),
    ];
    for text in texts {
        fs::write(&msg, &text).unwrap();
        let out = fx.gate_env(
            &fx.repo(),
            Some(&patterns),
            &["message", msg.to_str().unwrap()],
            &[("LC_ALL", "C.UTF-8")],
        );
        assert_eq!(
            out.status.code(),
            Some(3),
            "{:?}: {}",
            String::from_utf8_lossy(&text),
            show(&out)
        );
    }
}

/// A message file is a caller-asserted input, read from the caller's directory; a command line
/// wrong on its face is exit 2.
#[test]
fn message_and_range_arguments() {
    let fx = Fixture::new(&[("sub/m.txt", b"Fix the secretname import\n")]);
    let patterns = secret_list(&fx);
    let missing = fx.tmp.path().join("no-such-message");
    let out = fx.gate_in(
        &fx.repo(),
        Some(&patterns),
        &["message", missing.to_str().unwrap()],
    );
    assert_eq!(
        out.status.code(),
        Some(1),
        "a missing message: {}",
        show(&out)
    );

    let out = fx.gate_in(&fx.repo(), Some(&patterns), &["message", "sub"]);
    assert_eq!(out.status.code(), Some(1), "a directory: {}", show(&out));

    let out = fx.gate_in(
        &fx.repo().join("sub"),
        Some(&patterns),
        &["message", "m.txt"],
    );
    assert_eq!(
        out.status.code(),
        Some(3),
        "a relative message: {}",
        show(&out)
    );

    for args in [
        &["message"][..],
        &["message", ""][..],
        &["range"][..],
        &["range", "-p"][..],
        &["range", "--branches"][..],
        &["range", "--all", "x"][..],
        &["scan", "x"][..],
        &["message", "a", "b"][..],
        &["--help", "message", "a"][..],
    ] {
        let out = fx.gate_in(&fx.repo(), Some(&patterns), args);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {}", show(&out));
    }
}

/// `range` scans the message of each commit in the range, and of each annotated tag that points
/// to one of them, and only those.
#[test]
fn range_scans_commit_and_tag_messages() {
    let fx = Fixture::new(&[("README.md", b"clean\n")]);
    let patterns = secret_list(&fx);
    let base = fx.commit("Start");
    let leaky = fx.commit("Wire up secretname");
    let clean = fx.commit("Tidy");
    let gate = |range: &str, args: &[&str]| {
        let mut all = args.to_vec();
        all.extend(["range", range]);
        fx.gate_in(&fx.repo(), Some(&patterns), &all)
    };

    let out = gate(&format!("{base}..HEAD"), &[]);
    assert_eq!(out.status.code(), Some(3), "{}", show(&out));
    assert!(
        String::from_utf8_lossy(&out.stdout).contains(&format!("commit {}:1:", &leaky[..12])),
        "{}",
        show(&out)
    );
    let out = gate(&format!("{leaky}..HEAD"), &[]);
    assert_eq!(out.status.code(), Some(0), "past the leak: {}", show(&out));
    let out = gate("HEAD..HEAD", &[]);
    assert_eq!(out.status.code(), Some(0), "an empty range: {}", show(&out));

    let tag = fx.git_out(&["tag", "-a", "v1", "-m", "Release secretname", &clean]);
    assert!(tag.status.success(), "{}", show(&tag));
    let light = fx.git_out(&["tag", "light", &base]);
    assert!(light.status.success(), "{}", show(&light));
    let out = gate(&format!("{clean}^!"), &[]);
    assert_eq!(
        out.status.code(),
        Some(3),
        "a tag in the range: {}",
        show(&out)
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("tag v1:"),
        "{}",
        show(&out)
    );
    let out = gate(&format!("{base}^!"), &[]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "a tag outside it: {}",
        show(&out)
    );

    let home = fx.commit(&format!("Note {}", home_path()));
    let out = gate(&format!("{home}^!"), &["--generic-only"]);
    assert_eq!(
        out.status.code(),
        Some(3),
        "the generic layer: {}",
        show(&out)
    );
    let cited = fx.commit(&format!("Fix {}", id("bug", 2)));
    let out = gate(&format!("{cited}^!"), &["--generic-only"]);
    assert_eq!(
        out.status.code(),
        Some(3),
        "an ID with no floor: {}",
        show(&out)
    );

    let out = gate("no-such-rev..HEAD", &[]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "a revision git cannot find: {}",
        show(&out)
    );

    // A tag of a tag publishes both messages, though the inner one has no ref of its own.
    for args in [
        &["tag", "-a", "inner", "-m", "Inner names secretname", &base][..],
        &["tag", "-a", "outer", "-m", "Outer is clean", "inner"][..],
        &["tag", "-d", "inner"][..],
    ] {
        let out = fx.git_out(args);
        assert!(out.status.success(), "{args:?}: {}", show(&out));
    }
    let out = gate(&format!("{base}^!"), &[]);
    assert_eq!(out.status.code(), Some(3), "a nested tag: {}", show(&out));
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("tag outer:"),
        "{}",
        show(&out)
    );
}

/// `range --all` reads every ref's history, and every range reads the objects a push would send,
/// not their replacements.
#[test]
fn range_all_reads_every_ref_and_no_replacement() {
    let fx = Fixture::new(&[("README.md", b"clean\n")]);
    let patterns = secret_list(&fx);
    let base = fx.commit("Start");
    fx.git(&["checkout", "-q", "-b", "side"]);
    let side = fx.commit("Side work on secretname");
    fx.git(&["checkout", "-q", "-"]);

    let out = fx.gate_in(&fx.repo(), Some(&patterns), &["range", "HEAD"]);
    assert_eq!(out.status.code(), Some(0), "HEAD alone: {}", show(&out));
    let out = fx.gate_in(&fx.repo(), Some(&patterns), &["range", "--all"]);
    assert_eq!(out.status.code(), Some(3), "--all: {}", show(&out));

    let replace = fx.git_out(&["replace", &side, &base]);
    assert!(replace.status.success(), "{}", show(&replace));
    let out = fx.gate_in(
        &fx.repo(),
        Some(&patterns),
        &["range", &format!("{side}^!")],
    );
    assert_eq!(
        out.status.code(),
        Some(3),
        "a replaced commit: {}",
        show(&out)
    );
}

/// A NUL in a tag's message is a line break: bash would drop it and join the words around it.
#[test]
fn range_reads_a_nul_as_a_line_break() {
    let fx = Fixture::new(&[("README.md", b"clean\n")]);
    let base = fx.commit("Start");
    let tag = fx.tmp.path().join("tag");
    fs::write(
        &tag,
        format!(
            "object {base}\ntype commit\ntag nul\ntagger T <t@example.com> 0 +0000\n\nx\0{}\n",
            id("bug", 2)
        ),
    )
    .unwrap();
    let out = fx.git_out(&[
        "hash-object",
        "-t",
        "tag",
        "-w",
        "--literally",
        tag.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "{}", show(&out));
    let sha = String::from_utf8(out.stdout).unwrap().trim().to_string();
    fx.git(&["update-ref", "refs/tags/nul", &sha]);
    let out = fx.gate_in(
        &fx.repo(),
        None,
        &["--generic-only", "range", &format!("{base}^!")],
    );
    assert_eq!(out.status.code(), Some(3), "{}", show(&out));
}

/// A tag the gate cannot read stops it, and so does a shallow clone, whose history stops early:
/// neither may pass as clean.
#[test]
fn range_fails_closed_on_what_it_cannot_read() {
    let fx = Fixture::new(&[("README.md", b"clean\n")]);
    let base = fx.commit("Start");
    fx.commit("Next");
    let shallow = fx.tmp.path().join("shallow");
    let url = format!("file://{}", fx.repo().display());
    let out = fx
        .command("git", fx.tmp.path())
        .args([
            "clone",
            "-q",
            "--depth",
            "1",
            &url,
            shallow.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", show(&out));
    let out = fx.gate_in(&shallow, None, &["--generic-only", "range", "HEAD"]);
    assert_eq!(
        out.status.code(),
        Some(4),
        "a shallow clone: {}",
        show(&out)
    );

    for args in [
        &["tag", "-a", "inner", "-m", "Inner", &base][..],
        &["tag", "-a", "outer", "-m", "Outer", "inner"][..],
    ] {
        let out = fx.git_out(args);
        assert!(out.status.success(), "{args:?}: {}", show(&out));
    }
    let out = fx.git_out(&["rev-parse", "inner"]);
    let inner = String::from_utf8(out.stdout).unwrap().trim().to_string();
    fx.git(&["tag", "-d", "inner"]);
    let object = fx
        .repo()
        .join(".git/objects")
        .join(&inner[..2])
        .join(&inner[2..]);
    fs::remove_file(object).unwrap();
    let out = fx.gate_in(
        &fx.repo(),
        None,
        &["--generic-only", "range", &format!("{base}^!")],
    );
    assert_eq!(
        out.status.code(),
        Some(1),
        "an unreadable tag: {}",
        show(&out)
    );
}

/// The hook the gate is for: a commit whose message would leak is not made.
#[test]
fn a_commit_msg_hook_refuses_the_commit() {
    let fx = Fixture::new(&[("README.md", b"clean\n")]);
    fx.config_file("patterns.txt", "secretname\n");
    let hooks = fx.tmp.path().join("hooks");
    fs::create_dir(&hooks).unwrap();
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/leak-gate");
    let hook = hooks.join("commit-msg");
    fs::write(
        &hook,
        format!(
            "#!/bin/sh\nexec bash '{}' message \"$1\"\n",
            script.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    fx.git(&["config", "core.hooksPath", hooks.to_str().unwrap()]);

    let out = fx.git_out(&["commit", "-q", "-m", "Add secretname support"]);
    assert!(!out.status.success(), "{}", show(&out));
    let head = fx.git_out(&["rev-parse", "--verify", "--quiet", "HEAD"]);
    assert!(
        !head.status.success(),
        "the commit was made: {}",
        show(&head)
    );

    let out = fx.git_out(&[
        "commit",
        "-q",
        "-m",
        "Add support",
        "-m",
        "# secretname is the client",
    ]);
    assert!(
        !out.status.success(),
        "a `#` line, which -m keeps: {}",
        show(&out)
    );
    let head = fx.git_out(&["rev-parse", "--verify", "--quiet", "HEAD"]);
    assert!(
        !head.status.success(),
        "the commit was made: {}",
        show(&head)
    );

    let out = fx.git_out(&["commit", "-q", "-m", "Add support"]);
    assert!(out.status.success(), "a clean message: {}", show(&out));
}

/// In a UTF-8 locale, git grep and macOS's grep match nothing on a line holding a byte that is not
/// UTF-8, so a private name or a home path there passed the index scan.
#[test]
fn the_index_scan_reads_a_line_that_is_not_utf8() {
    let home = [
        b"see \xff ".to_vec(),
        home_path().into_bytes(),
        b"\n".to_vec(),
    ]
    .concat();
    for (file, contents) in [
        ("a.txt", b"Fix \xff secretname\n".to_vec()),
        ("b.txt", home),
    ] {
        let fx = Fixture::new(&[(file, &contents[..])]);
        let patterns = secret_list(&fx);
        for locale in ["C.UTF-8", "en_US.UTF-8"] {
            let out = fx.gate_env(&fx.repo(), Some(&patterns), &[], &[("LC_ALL", locale)]);
            assert_eq!(
                out.status.code(),
                Some(3),
                "{file}, {locale}: {}",
                show(&out)
            );
        }
    }
}
