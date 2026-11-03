//! `scripts/leak-gate`, run against scratch repositories.
//!
//! The script's header is its contract: it scans every tracked file, except the paths a tracked
//! `.leak-gate-exclude` lists, matching private names, and bug or plan IDs below their series'
//! floor, in a file's contents and in its path; exit 1 means "the gate itself failed", and 3 means
//! findings.
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

    /// The gate run in `dir`, with `LEAK_GATE_PATTERNS` set to `patterns` when it is given.
    fn gate_in(&self, dir: &Path, patterns: Option<&Path>, args: &[&str]) -> Output {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/leak-gate");
        let mut c = self.command("bash", dir);
        c.arg(script).args(args);
        if let Some(p) = patterns {
            c.env("LEAK_GATE_PATTERNS", p);
        }
        c.output().unwrap()
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
