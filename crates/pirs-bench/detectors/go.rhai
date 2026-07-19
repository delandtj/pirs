// Go detector. `gotestsum` emits JUnit natively; the probe (`go test -list`)
// confirms the toolchain. Because every candidate is probe-confirmed and every
// run must produce JUnit, shipping this best-effort is safe: if gotestsum is
// absent or the id format is wrong for a given repo, the run simply fails to
// produce JUnit and the driver surfaces it — it never yields a false pass.
//
// Refinement note: Go's `-run` takes a regex, so multiple target ids should be
// joined with `|` rather than spaces. The single space-joined `{tests}` fits
// pytest exactly but is a known approximation here; a per-framework id join is
// a later enhancement.
fn detect() {
    let specs = [];
    if path_exists("go.mod") {
        specs.push(#{
            framework: "go",
            install: [],
            list_cmd: "go test -list \".*\" ./... 2>/dev/null | head -n 5",
            test_cmd: "gotestsum --junitfile {junit} --format standard-quiet -- -run \"{tests}\" ./...",
            timeout_secs: 900,
            parallel: true,
        });
    }
    specs
}
