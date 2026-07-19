// pytest detector. The `--junitxml=PATH` flag is stable and universal, and a
// pytest node id (`path/to/test.py::TestClass::test_case`) is exactly what the
// space-joined `{tests}` placeholder needs — so this detector is a clean fit.
//
// The SWE-bench-heavy repos (django, sympy, sphinx, pytest itself) are all
// Python, so this is the flagship detector.
fn detect() {
    let specs = [];
    let is_python =
        path_exists("pyproject.toml") || path_exists("setup.py") ||
        path_exists("setup.cfg")     || path_exists("tox.ini")  ||
        path_exists("conftest.py");
    if is_python {
        specs.push(#{
            framework: "pytest",
            // Editable install is the usual prerequisite; tolerate its absence so
            // the probe still runs and surfaces the real error if it matters.
            install: ["pip install -e . 2>/dev/null || true"],
            // Collect-only probe: proves pytest can import and enumerate tests.
            list_cmd: "python -m pytest --collect-only -q 2>/dev/null | head -n 5",
            // Space-joined node ids in {tests}; empty {tests} runs the whole suite.
            test_cmd: "python -m pytest {tests} --junitxml={junit} -p no:cacheprovider -q",
            timeout_secs: 1200,
            parallel: false,
        });
    }
    specs
}
