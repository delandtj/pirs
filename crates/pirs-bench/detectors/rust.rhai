// Rust detector. cargo-nextest is much faster than `cargo test` and emits JUnit
// via a profile whose path is set in `.config/nextest.toml`. Since the host file
// API is read-only, the test command writes that config from the shell before
// running, then copies the emitted report to {junit}.
//
// This is best-effort and probe-gated: `cargo nextest list` confirms nextest is
// present; if the config/copy incantation is wrong for a given repo, the run
// produces no JUnit and the driver surfaces it — never a false pass.
//
// Refinement note: nextest selects tests by substring/filterset, not the same id
// shape as pytest; multi-target id formatting is a later enhancement.
fn detect() {
    let specs = [];
    if path_exists("Cargo.toml") {
        specs.push(#{
            framework: "rust",
            install: [],
            list_cmd: "cargo nextest list 2>/dev/null | head -n 5",
            test_cmd:
                "mkdir -p .config && printf '[profile.pirs.junit]\\npath = \"junit.xml\"\\n' > .config/nextest.toml && " +
                "cargo nextest run --profile pirs {tests} ; " +
                "cp target/nextest/pirs/junit.xml {junit} 2>/dev/null",
            timeout_secs: 1800,
            parallel: true,
        });
    }
    specs
}
