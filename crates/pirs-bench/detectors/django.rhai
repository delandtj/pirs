// Django detector for the official test suite layout used by SWE-bench
// (`tests/runtests.py` + a top-level `django/` package).
//
// Why this exists: the generic pytest detector never confirms on django images
// (unittest labels, not pytest node ids), so every django instance fell through
// to agent-discovered self-report — ReproFailed / FixNoFlip / turns=0 (see
// fifty_pirs LEARNINGS F1–F4). A real runner lets baseline + verify execute
// `runtests.py` and emit JUnit the harness already understands.
//
// Id format: SWE-bench FAIL_TO_PASS uses unittest labels
//   `test_foo (module.Class)`
// which runtests accepts as
//   `module.Class.test_foo`
// The embedded wrapper converts and parses `-v2` text into JUnit XML.
//
// Settings: SWE-bench django images ship `test_sqlite`; that is the documented
// default for the suite when DJANGO_SETTINGS_MODULE is unset, and we pass it
// explicitly for determinism.

fn detect() {
    let specs = [];
    // Structural markers: the django source tree + the suite driver.
    if path_exists("tests/runtests.py") && path_exists("django") {
        let py = "PY=$(command -v python3 || command -v python); ";
        // Base64 of pirs_django_run.py (keeps rhai free of nested-quote hell).
        // Source lives next to this comment in git history; regenerate with:
        //   python3 -c 'import base64,pathlib;print(base64.b64encode(pathlib.Path("wrap.py").read_bytes()).decode())'
        let b64 = "aW1wb3J0IHJlLHN5cyxzdWJwcm9jZXNzLHhtbC5ldHJlZS5FbGVtZW50VHJlZSBhcyBFVApkZWYgbGFiKHQpOgogICAgdD10LnN0cmlwKCkKICAgIG09cmUubWF0Y2gociJeKHRlc3RfXHcrKVxzK1woKC4rKVwpJCIsdCkKICAgIGlmIG06CiAgICAgICAgcmV0dXJuICIlcy4lcyIgJSAobS5ncm91cCgyKSwgbS5ncm91cCgxKSkKICAgIHJldHVybiB0Cmlkcz1zeXMuYXJndlsxOi0xXQpqdW5pdD1zeXMuYXJndlstMV0KaWYgbm90IGlkczoKICAgIG9wZW4oanVuaXQsIndiIikud3JpdGUoYic8dGVzdHN1aXRlIG5hbWU9ImRqYW5nbyIvPicpCiAgICBzeXMuZXhpdCgwKQpsYWJlbHM9W2xhYihpKSBmb3IgaSBpbiBpZHNdCmNtZD1bc3lzLmV4ZWN1dGFibGUsInRlc3RzL3J1bnRlc3RzLnB5IiwiLS1zZXR0aW5ncz10ZXN0X3NxbGl0ZSIsIi0tcGFyYWxsZWwiLCIxIiwiLXYyIl0rbGFiZWxzCnA9c3VicHJvY2Vzcy5ydW4oY21kLHN0ZG91dD1zdWJwcm9jZXNzLlBJUEUsc3RkZXJyPXN1YnByb2Nlc3MuUElQRSx1bml2ZXJzYWxfbmV3bGluZXM9VHJ1ZSkKdGV4dD0ocC5zdGRvdXQgb3IgIiIpKyJcbiIrKHAuc3RkZXJyIG9yICIiKQpvdXRjb21lcz17fQojIFBhc3NlczogInRlc3RfeCAobW9kLkNsYXNzKSAuLi4gb2siIChvayBtdXN0IGltbWVkaWF0ZWx5IGZvbGxvdyAuLi4pCmZvciBtIGluIHJlLmZpbmRpdGVyKHIiKFxTKylccytcKChbXildKylcKVxzK1wuXC5cLlxzK29rXGIiLCB0ZXh0LCByZS5JKToKICAgIG91dGNvbWVzWyhtLmdyb3VwKDIpLCBtLmdyb3VwKDEpKV0gPSAicGFzcyIKIyBFeHBsaWNpdCBmYWlsL2Vycm9yIG9uIHByb2dyZXNzIGxpbmUKZm9yIG0gaW4gcmUuZmluZGl0ZXIociIoXFMrKVxzK1woKFteKV0rKVwpXHMrXC5cLlwuXHMrKEZBSUx8RVJST1IpXGIiLCB0ZXh0LCByZS5JKToKICAgIHN0ID0gImZhaWwiIGlmIG0uZ3JvdXAoMykubG93ZXIoKT09ImZhaWwiIGVsc2UgImVycm9yIgogICAgb3V0Y29tZXNbKG0uZ3JvdXAoMiksIG0uZ3JvdXAoMSkpXSA9IHN0CiMgU3VtbWFyeSBibG9ja3MgKGF1dGhvcml0YXRpdmUgZm9yIGZhaWx1cmVzOyBjb3ZlcnMgc3ViVGVzdCBub2lzZSkKZm9yIG0gaW4gcmUuZmluZGl0ZXIociJeKEVSUk9SfEZBSUwpOlxzKyhcUyspXHMrXCgoW14pXSspXCkiLCB0ZXh0LCByZS5JfHJlLk0pOgogICAgc3QgPSAiZXJyb3IiIGlmIG0uZ3JvdXAoMSkubG93ZXIoKT09ImVycm9yIiBlbHNlICJmYWlsIgogICAgb3V0Y29tZXNbKG0uZ3JvdXAoMyksIG0uZ3JvdXAoMikpXSA9IHN0CnJvb3Q9RVQuRWxlbWVudCgidGVzdHN1aXRlIixuYW1lPSJkamFuZ28iKQpmb3IgdGlkIGluIGlkczoKICAgIG09cmUubWF0Y2gociJeKHRlc3RfXHcrKVxzK1woKC4rKVwpJCIsIHRpZC5zdHJpcCgpKQogICAgaWYgbToKICAgICAgICBuYW1lLGNscz1tLmdyb3VwKDEpLG0uZ3JvdXAoMikKICAgIGVsc2U6CiAgICAgICAgbmFtZSxjbHM9dGlkLCJ1bmtub3duIgogICAgc3Q9b3V0Y29tZXMuZ2V0KChjbHMsbmFtZSkpCiAgICBpZiBzdCBpcyBOb25lOgogICAgICAgIGZvciAoYyxuKSxvIGluIG91dGNvbWVzLml0ZW1zKCk6CiAgICAgICAgICAgIGlmIG49PW5hbWU6CiAgICAgICAgICAgICAgICBzdCxvX2Nscz1vLGMKICAgICAgICAgICAgICAgIGNscz1jCiAgICAgICAgICAgICAgICBicmVhawogICAgdGM9RVQuU3ViRWxlbWVudChyb290LCJ0ZXN0Y2FzZSIsY2xhc3NuYW1lPWNscyxuYW1lPW5hbWUpCiAgICBpZiBzdCBpcyBOb25lOgogICAgICAgIEVULlN1YkVsZW1lbnQodGMsInNraXBwZWQiKS50ZXh0PSJub3Qgc2VlbiBpbiBydW5uZXIgb3V0cHV0IgogICAgZWxpZiBzdD09InBhc3MiOgogICAgICAgIHBhc3MKICAgIGVsaWYgc3Q9PSJmYWlsIjoKICAgICAgICBFVC5TdWJFbGVtZW50KHRjLCJmYWlsdXJlIikudGV4dD10aWQKICAgIGVsaWYgc3Q9PSJlcnJvciI6CiAgICAgICAgRVQuU3ViRWxlbWVudCh0YywiZXJyb3IiKS50ZXh0PXRpZAogICAgZWxzZToKICAgICAgICBFVC5TdWJFbGVtZW50KHRjLCJza2lwcGVkIikudGV4dD10aWQKRVQuRWxlbWVudFRyZWUocm9vdCkud3JpdGUoanVuaXQsIGVuY29kaW5nPSJ1dGYtOCIsIHhtbF9kZWNsYXJhdGlvbj1UcnVlKQpzeXMuZXhpdCgwKQo=";
        specs.push(#{
            framework: "django",
            // SWE-bench images already have django + deps in the testbed env.
            install: [],
            // Cheap confirmation: import django and prove the suite driver exists.
            // Probe requires ≥1 non-empty stdout line.
            list_cmd: py + "test -f tests/runtests.py && \"$PY\" -c \"import django; print('django', django.get_version())\"",
            // Decode wrapper once per run, then pass shell-quoted unittest labels
            // plus the junit path. shell_quote_tests protects spaces/parens in ids.
            test_cmd: py + "echo " + b64 + " | base64 -d > /tmp/pirs_django_run.py && \"$PY\" /tmp/pirs_django_run.py {tests} {junit}",
            shell_quote_tests: true,
            // Individual django tests often spin up a DB; keep budget generous.
            timeout_secs: 1800,
            parallel: false,
        });
    }
    specs
}
