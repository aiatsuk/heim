//! Pins the `heim.stats.v1` JSON contract.
//!
//! This schema is heim's public API for agents: `docs/for-agents.md` tabulates
//! the top-level fields and the member names of `loc` / `size` / `git` /
//! `deltas[]` / `session` / `history`, and `skills/heim-audit/SKILL.md` tells
//! agents to key off `deltas[].code`, `deltas[].ready`, `hints[]` and
//! `git.recent_commits`.
//!
//! Nothing else enforces it — the unit tests assert three substrings, so a
//! rename would ship green and silently break every consumer. These tests run
//! the real binary against a real git repository and check the emitted JSON.

// `clippy.toml` bans bare `Command::output()` because it can block a collector
// thread forever. That invariant is about heim's runtime; this is a test
// harness driving the finished binary, with no TUI to wedge and no access to
// `collect::run_timed` (heim is a binary crate). A hang here fails the suite,
// which is the desired outcome anyway.
#![expect(clippy::disallowed_methods, reason = "test harness, not a collector")]

use std::path::{Path, PathBuf};
use std::process::Command;

fn heim_bin() -> PathBuf {
    // `CARGO_BIN_EXE_<name>` is set by cargo for integration tests.
    PathBuf::from(env!("CARGO_BIN_EXE_heim"))
}

fn sh(dir: &Path, program: &str, args: &[&str]) {
    let out = Command::new(program)
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("spawn {program}: {e}"));
    assert!(
        out.status.success(),
        "{program} {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A throwaway git repo with one commit, so `git` and `deltas[].insertions`
/// are populated rather than null.
fn fixture(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("heim-contract-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.rs"), "fn main() {\n    // hello\n}\n").unwrap();
    std::fs::write(dir.join("notes.md"), "# notes\n\ntext\n").unwrap();
    sh(&dir, "git", &["init", "-q", "."]);
    sh(&dir, "git", &["add", "-A"]);
    sh(
        &dir,
        "git",
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-qm",
            "initial",
        ],
    );
    dir
}

fn report(dir: &Path) -> serde_json::Value {
    let out = Command::new(heim_bin())
        .args(["--once", "--json"])
        .arg(dir)
        .output()
        .expect("run heim");
    assert!(
        out.status.success(),
        "heim failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("stdout must be valid JSON")
}

#[test]
fn stats_v1_shape_is_stable() {
    let dir = fixture("shape");
    let v = report(&dir);

    assert_eq!(v["schema"], "heim.stats.v1");

    for key in [
        "schema",
        "version",
        "purpose",
        "path",
        "collected_at",
        "collected_at_unix",
        "took_secs",
        "loc",
        "size",
        "git",
        "deltas",
        "session",
        "history",
        "hints",
    ] {
        assert!(!v[key].is_null(), "missing top-level field `{key}`");
    }

    for key in ["code", "files", "blank", "comment", "languages"] {
        assert!(!v["loc"][key].is_null(), "missing loc.{key}");
    }
    for key in ["bytes", "human", "engine", "top"] {
        assert!(!v["size"][key].is_null(), "missing size.{key}");
    }
    for key in [
        "branch",
        "working_tree_insertions",
        "working_tree_deletions",
        "recent_commits",
    ] {
        assert!(!v["git"][key].is_null(), "missing git.{key}");
    }
    for key in [
        "code_delta",
        "size_bytes_delta",
        "samples_loaded_from_store",
    ] {
        assert!(v["session"].get(key).is_some(), "missing session.{key}");
    }
    for key in ["samples", "span_secs", "oldest_unix", "newest_unix"] {
        assert!(v["history"].get(key).is_some(), "missing history.{key}");
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn deltas_cover_documented_windows_and_never_lie() {
    let dir = fixture("deltas");
    let v = report(&dir);

    let deltas = v["deltas"].as_array().expect("deltas must be an array");
    let windows: Vec<&str> = deltas
        .iter()
        .map(|d| d["window"].as_str().unwrap())
        .collect();
    assert_eq!(
        windows,
        ["5m", "10m", "30m", "1h", "2h", "4h", "8h", "1d"],
        "documented delta windows changed"
    );

    for d in deltas {
        for key in [
            "window",
            "window_secs",
            "ready",
            "code",
            "size_bytes",
            "insertions",
            "deletions",
        ] {
            assert!(d.get(key).is_some(), "missing deltas[].{key} in {d}");
        }
        // The contract that matters: a window that is not ready must not report
        // a number an agent would read as "nothing was written".
        if !d["ready"].as_bool().unwrap() {
            assert!(
                d["code"].is_null(),
                "window {} is not ready but reports code={} — indistinguishable \
                 from 'no growth'",
                d["window"],
                d["code"]
            );
        }
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn languages_and_commits_are_populated_and_typed() {
    let dir = fixture("langs");
    let v = report(&dir);

    let langs = v["loc"]["languages"].as_array().unwrap();
    assert!(!langs.is_empty(), "fixture has Rust and Markdown sources");
    for l in langs {
        for key in ["name", "code", "blank", "comment", "pct"] {
            assert!(l.get(key).is_some(), "missing languages[].{key}");
        }
        assert!(l["name"].is_string());
        assert!(l["code"].is_u64());
    }

    let commits = v["git"]["recent_commits"].as_array().unwrap();
    assert_eq!(commits.len(), 1, "fixture has exactly one commit");
    for key in ["short", "subject", "author", "insertions", "deletions"] {
        assert!(
            commits[0].get(key).is_some(),
            "missing recent_commits[].{key}"
        );
    }
    assert_eq!(commits[0]["subject"], "initial");

    assert!(
        v["hints"]
            .as_array()
            .map(|h| !h.is_empty())
            .unwrap_or(false),
        "agents are told to read hints[]"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// `-o FILE` writes the file and stays silent on stdout; `-o -` is stdout only.
/// Both are promised by `--help` and `docs/for-agents.md`.
#[test]
fn output_flag_routing_matches_docs() {
    let dir = fixture("output");
    let target = dir.join("report.json");

    let out = Command::new(heim_bin())
        .args(["--once", "--json", "-o"])
        .arg(&target)
        .arg(&dir)
        .output()
        .expect("run heim");
    assert!(out.status.success());
    assert!(
        out.stdout.is_empty(),
        "-o FILE must not also dump the report to stdout"
    );
    let written = std::fs::read(&target).expect("-o FILE must write the file");
    let parsed: serde_json::Value = serde_json::from_slice(&written).unwrap();
    assert_eq!(parsed["schema"], "heim.stats.v1");

    let out = Command::new(heim_bin())
        .args(["--once", "--json", "-o", "-"])
        .arg(&dir)
        .output()
        .expect("run heim");
    assert!(out.status.success());
    assert!(!out.stdout.is_empty(), "-o - must print to stdout");

    let _ = std::fs::remove_dir_all(&dir);
}
