//! AS-019 — each subcommand surfaces its own --help with examples.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_graphatlas")
}

fn help_of(subcmd: &str) -> String {
    let out = Command::new(bin())
        .arg(subcmd)
        .arg("--help")
        .output()
        .expect("spawn");
    assert!(out.status.success(), "{subcmd} --help failed: {:?}", out);
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn each_subcommand_has_dedicated_help() {
    for sub in [
        "mcp", "init", "doctor", "install", "list", "bench", "update", "cache",
    ] {
        let h = help_of(sub);
        assert!(
            h.to_uppercase().contains("EXAMPLES") || h.contains("graphatlas"),
            "{sub} --help missing EXAMPLES section:\n{h}"
        );
    }
}

#[test]
fn install_help_lists_all_three_clients() {
    let h = help_of("install");
    assert!(h.contains("claude"), "{h}");
    assert!(h.contains("cursor"), "{h}");
    assert!(h.contains("cline"), "{h}");
}

#[test]
fn doctor_help_lists_five_checks() {
    let h = help_of("doctor");
    for label in ["Binary", "MCP config", "entry", "Cache dir", "Fixture"] {
        assert!(h.contains(label), "doctor --help missing `{label}`:\n{h}");
    }
}

#[test]
fn mcp_help_mentions_2025_11_25_spec() {
    let h = help_of("mcp");
    assert!(
        h.contains("2025-11-25"),
        "mcp --help should reference MCP spec 2025-11-25: {h}"
    );
}

#[test]
fn update_help_notes_v1_1_deferral() {
    let h = help_of("update");
    assert!(
        h.contains("v1.1") || h.to_lowercase().contains("deferred"),
        "update --help should note v1.1 deferral: {h}"
    );
}

#[test]
fn root_help_lists_all_eight_subcommands() {
    let out = Command::new(bin()).arg("--help").output().unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    for sub in [
        "mcp", "init", "doctor", "install", "list", "bench", "update", "cache",
    ] {
        assert!(s.contains(sub), "root help missing `{sub}`:\n{s}");
    }
}
