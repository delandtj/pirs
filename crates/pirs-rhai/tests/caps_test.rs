use std::sync::Arc;

use pirs_rhai::ExtensionHost;

const SCRIPT: &str = r#"// caps: {"exec": ["git"], "fs": ["./.pirs/**"], "subagents": 0}

fn probe(what) {
    if what == "exec-ok" { return exec("git --version", 5); }
    if what == "exec-blocked" { return exec("rm -rf /tmp/nope", 5); }
    if what == "exec-metachar" { return exec("git status && rm x", 5); }
    if what == "fs-ok" { return fs_write(".pirs/caps-test.txt", "hi"); }
    if what == "fs-blocked" { return fs_write("/tmp/caps-blocked.txt", "hi"); }
    if what == "subagent" { return run_subagent("task"); }
    ()
}
"#;

fn call(host: &Arc<ExtensionHost>, what: &str) -> rhai::Dynamic {
    let idx = 0;
    host.call_extension_for_test(idx, "probe", (what.to_string(),))
        .unwrap()
}

#[test]
fn caps_enforced_at_host_fn_boundary() {
    let tmp = tempfile::tempdir().unwrap();
    let prev = std::env::current_dir().unwrap();
    std::env::set_current_dir(tmp.path()).unwrap();

    let mut host = ExtensionHost::new();
    host.set_subagent_runner(Arc::new(|_, _| Ok("ran".to_string())));
    host.load_source(SCRIPT, "capped.rhai".into()).unwrap();
    let host = Arc::new(host);

    // exec allowlist: git runs, rm is blocked, metachars are blocked.
    let ok = call(&host, "exec-ok");
    let code: rhai::INT = ok.read_lock::<rhai::Map>().unwrap()["code"]
        .as_int()
        .unwrap();
    assert_eq!(code, 0, "git should run");

    let blocked = call(&host, "exec-blocked");
    let map = blocked.read_lock::<rhai::Map>().unwrap();
    assert_eq!(map["code"].as_int().unwrap(), -1);
    assert!(map["output"].to_string().contains("not in exec allowlist"));

    let meta = call(&host, "exec-metachar");
    let map = meta.read_lock::<rhai::Map>().unwrap();
    assert_eq!(map["code"].as_int().unwrap(), -1);
    assert!(map["output"].to_string().contains("metacharacter"));

    // fs allowlist: inside prefix works, outside is refused.
    assert!(call(&host, "fs-ok").as_bool().unwrap());
    assert!(!call(&host, "fs-blocked").as_bool().unwrap());
    assert!(!std::path::Path::new("/tmp/caps-blocked.txt").exists());

    // subagents: 0 denies run_subagent.
    let sub = call(&host, "subagent").cast::<String>();
    assert!(sub.contains("denied by capability manifest"), "{sub}");

    std::env::set_current_dir(prev).unwrap();
}

#[test]
fn no_manifest_stays_unrestricted() {
    let mut host = ExtensionHost::new();
    host.load_source(
        "fn probe() { fs_write(\"/tmp/caps-free.txt\", \"x\") }",
        "free.rhai".into(),
    )
    .unwrap();
    let host = Arc::new(host);
    let r = host.call_extension_for_test(0, "probe", ()).unwrap();
    assert!(r.as_bool().unwrap());
    let _ = std::fs::remove_file("/tmp/caps-free.txt");
}
