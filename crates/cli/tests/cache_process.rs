use std::fs;
use std::process::Command;

#[test]
fn independent_processes_serialize_cache_read_merge_write() {
    let root = std::env::temp_dir().join(format!("wae-cache-process-{}", std::process::id()));
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/a.ts"), "export const value = 1;").unwrap();
    fs::write(
        root.join("wae.yaml"),
        "version: 1\narchitecture:\n  layers: {}\ncache:\n  enabled: true\n  directory: .wae/cache\n",
    )
    .unwrap();

    let spawn = || {
        Command::new(env!("CARGO_BIN_EXE_wae-cli")).arg("check").current_dir(&root).spawn().unwrap()
    };
    let mut first = spawn();
    let mut second = spawn();
    assert!(first.wait().unwrap().success());
    assert!(second.wait().unwrap().success());

    let cache = fs::read_to_string(root.join(".wae/cache/imports-v1.json")).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&cache).unwrap();
    assert!(parsed["files"]["src/a.ts"].is_object());
    fs::remove_dir_all(root).unwrap();
}
