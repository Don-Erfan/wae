use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use serde_json::{Value, json};
use url::Url;

fn send(stdin: &mut impl Write, message: &Value) {
    let body = serde_json::to_vec(message).unwrap();
    write!(stdin, "Content-Length: {}\r\n\r\n", body.len()).unwrap();
    stdin.write_all(&body).unwrap();
    stdin.flush().unwrap();
}

fn read_message(reader: &mut BufReader<impl Read>) -> Option<Value> {
    let mut length = None;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).unwrap() == 0 {
            return None;
        }
        if header == "\r\n" {
            break;
        }
        if let Some(value) = header.strip_prefix("Content-Length:") {
            length = Some(value.trim().parse::<usize>().unwrap());
        }
    }
    let mut body = vec![0; length.expect("LSP Content-Length")];
    reader.read_exact(&mut body).unwrap();
    Some(serde_json::from_slice(&body).unwrap())
}

fn message_receiver(stdout: ChildStdout) -> Receiver<Value> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        while let Some(message) = read_message(&mut reader) {
            if sender.send(message).is_err() {
                break;
            }
        }
    });
    receiver
}

fn receive(receiver: &Receiver<Value>) -> Value {
    receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("wae-lsp did not produce the expected protocol message within 30 seconds")
}

#[test]
fn stdio_server_publishes_diagnostics_and_shuts_down_cleanly() {
    let root = std::env::temp_dir().join(format!("wae-lsp-e2e-{}", std::process::id()));
    fs::create_dir_all(root.join("src")).unwrap();
    let root = root.canonicalize().unwrap();
    fs::write(root.join("src/a.ts"), "import './missing';").unwrap();
    fs::write(root.join("wae.yaml"), "version: 1\nresolution:\n  mode: bundler\n").unwrap();
    let root_uri = Url::from_directory_path(&root).unwrap().to_string();
    let mut child = Command::new(env!("CARGO_BIN_EXE_wae-lsp"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let stdout = message_receiver(child.stdout.take().unwrap());

    send(
        &mut stdin,
        &json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":root_uri}}),
    );
    let initialized = receive(&stdout);
    assert_eq!(initialized["id"], 1);
    send(&mut stdin, &json!({"jsonrpc":"2.0","method":"initialized","params":{}}));

    let published = loop {
        let message = receive(&stdout);
        if message["method"] == "textDocument/publishDiagnostics" {
            break message;
        }
    };
    assert!(
        published["params"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| { diagnostic["code"] == "RESOLVE-001" })
    );
    let diagnostic = published["params"]["diagnostics"][0].clone();
    let document_uri = Url::from_file_path(root.join("src/a.ts")).unwrap().to_string();
    send(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0", "id":4, "method":"textDocument/codeAction",
            "params":{
                "textDocument":{"uri":document_uri.clone()},
                "range":diagnostic["range"],
                "context":{"diagnostics":[diagnostic]}
            }
        }),
    );
    let actions = receive(&stdout);
    assert_eq!(actions["id"], 4);
    assert_eq!(actions["result"].as_array().unwrap().len(), 3);
    assert_eq!(actions["result"][0]["command"]["command"], "wae.explainRule");
    assert_eq!(
        actions["result"][2]["edit"]["changes"][&document_uri][0]["newText"],
        "// wae-ignore RESOLVE-001 -- \n"
    );
    send(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0", "id":6, "method":"workspace/executeCommand",
            "params":{"command":"wae.explainRule","arguments":[{"ruleId":"RESOLVE-001","notify":false}]}
        }),
    );
    let explained = receive(&stdout);
    assert_eq!(explained["id"], 6);
    assert!(explained["result"]["markdown"].as_str().unwrap().contains("### How to fix"));
    send(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0", "id":7, "method":"workspace/executeCommand",
            "params":{"command":"wae.inspectModule","arguments":[{"uri":document_uri.clone()}]}
        }),
    );
    let inspected = receive(&stdout);
    assert_eq!(inspected["id"], 7);
    assert_eq!(inspected["result"]["id"], "src/a.ts");
    send(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0", "id":8, "method":"workspace/executeCommand",
            "params":{"command":"wae.architectureOverview","arguments":[]}
        }),
    );
    let overview = receive(&stdout);
    assert_eq!(overview["id"], 8);
    let workspace = &overview["result"]["workspaces"][0];
    assert_eq!(workspace["ready"], true);
    assert!(
        workspace["overview"]["violations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|group| group["ruleId"] == "RESOLVE-001")
    );
    send(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0", "id":5, "method":"workspace/executeCommand",
            "params":{"command":"wae.showSuggestion","arguments":[{"suggestion":"Resolve the missing module."}]}
        }),
    );
    let suggestion = receive(&stdout);
    assert_eq!(suggestion["method"], "window/showMessage");
    assert_eq!(suggestion["params"]["message"], "Resolve the missing module.");
    assert_eq!(receive(&stdout)["id"], 5);

    send(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0", "method":"textDocument/didOpen",
            "params":{"textDocument":{"uri":document_uri.clone(),"languageId":"typescript","version":1,"text":"import './missing';"}}
        }),
    );
    for version in 2..=25 {
        let text = if version == 25 { "export const fixed = true;" } else { "import './missing';" };
        send(
            &mut stdin,
            &json!({
                "jsonrpc":"2.0", "method":"textDocument/didChange",
                "params":{"textDocument":{"uri":document_uri.clone(),"version":version},"contentChanges":[{"text":text}]}
            }),
        );
    }
    let settled = loop {
        let message = receive(&stdout);
        if message["method"] == "textDocument/publishDiagnostics"
            && message["params"]["uri"] == document_uri
            && message["params"]["diagnostics"].as_array().is_some_and(Vec::is_empty)
        {
            break message;
        }
    };
    assert_eq!(settled["params"]["diagnostics"], json!([]));

    send(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0", "id":3, "method":"textDocument/hover",
            "params":{"textDocument":{"uri":document_uri.clone()},"position":{"line":0,"character":0}}
        }),
    );
    let hover = loop {
        let message = receive(&stdout);
        if message["id"] == 3 {
            break message;
        }
    };
    assert!(hover["result"]["contents"]["value"].as_str().unwrap().contains("WAE architecture"));

    send(&mut stdin, &json!({"jsonrpc":"2.0","id":2,"method":"shutdown","params":null}));
    send(&mut stdin, &json!({"jsonrpc":"2.0","method":"exit","params":null}));
    assert_eq!(receive(&stdout)["id"], 2);
    drop(stdin);
    assert!(child.wait().unwrap().success());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn stdio_server_routes_multiple_workspaces_and_uses_utf16_positions() {
    let parent = std::env::temp_dir().join(format!("wae-lsp-multi-{}", std::process::id()));
    let clean = parent.join("clean");
    let broken = parent.join("broken");
    fs::create_dir_all(clean.join("src")).unwrap();
    fs::create_dir_all(broken.join("src")).unwrap();
    let parent = parent.canonicalize().unwrap();
    let clean = clean.canonicalize().unwrap();
    let broken = broken.canonicalize().unwrap();
    fs::write(clean.join("src/index.ts"), "export const clean = true;").unwrap();
    let source = "const emoji = \"😀\"; import './missing';";
    fs::write(broken.join("src/index.ts"), source).unwrap();
    for root in [&clean, &broken] {
        fs::write(root.join("wae.yaml"), "version: 1\nresolution:\n  mode: bundler\n").unwrap();
    }
    let broken_uri = Url::from_file_path(broken.join("src/index.ts")).unwrap().to_string();
    let mut child = Command::new(env!("CARGO_BIN_EXE_wae-lsp"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let stdout = message_receiver(child.stdout.take().unwrap());
    send(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0", "id":1, "method":"initialize",
            "params":{"workspaceFolders":[
                {"name":"clean","uri":Url::from_directory_path(&clean).unwrap().to_string()},
                {"name":"broken","uri":Url::from_directory_path(&broken).unwrap().to_string()}
            ]}
        }),
    );
    let initialized = receive(&stdout);
    assert_eq!(initialized["result"]["capabilities"]["positionEncoding"], "utf-16");
    send(&mut stdin, &json!({"jsonrpc":"2.0","method":"initialized","params":{}}));

    let diagnostic = loop {
        let message = receive(&stdout);
        if message["method"] == "textDocument/publishDiagnostics"
            && message["params"]["uri"] == broken_uri
            && message["params"]["diagnostics"].as_array().is_some_and(|items| !items.is_empty())
        {
            break message["params"]["diagnostics"][0].clone();
        }
    };
    assert_eq!(diagnostic["code"], "RESOLVE-001");
    let specifier = source.find("./missing").unwrap();
    assert_eq!(
        diagnostic["range"]["start"]["character"],
        source[..specifier].encode_utf16().count()
    );

    let added = parent.join("added");
    fs::create_dir_all(added.join("src")).unwrap();
    let added = added.canonicalize().unwrap();
    fs::write(added.join("src/index.ts"), "import './missing';").unwrap();
    fs::write(added.join("wae.yaml"), "version: 1\n").unwrap();
    let added_root_uri = Url::from_directory_path(&added).unwrap().to_string();
    let added_file_uri = Url::from_file_path(added.join("src/index.ts")).unwrap().to_string();
    send(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0", "method":"workspace/didChangeWorkspaceFolders",
            "params":{"event":{"added":[{"name":"added","uri":added_root_uri.clone()}],"removed":[]}}
        }),
    );
    loop {
        let message = receive(&stdout);
        if message["method"] == "textDocument/publishDiagnostics"
            && message["params"]["uri"] == added_file_uri
            && message["params"]["diagnostics"].as_array().is_some_and(|items| !items.is_empty())
        {
            break;
        }
    }
    send(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0", "method":"workspace/didChangeWorkspaceFolders",
            "params":{"event":{"added":[],"removed":[{"name":"added","uri":added_root_uri}]}}
        }),
    );
    loop {
        let message = receive(&stdout);
        if message["method"] == "textDocument/publishDiagnostics"
            && message["params"]["uri"] == added_file_uri
            && message["params"]["diagnostics"] == json!([])
        {
            break;
        }
    }

    send(&mut stdin, &json!({"jsonrpc":"2.0","id":2,"method":"shutdown","params":null}));
    send(&mut stdin, &json!({"jsonrpc":"2.0","method":"exit","params":null}));
    while receive(&stdout)["id"] != 2 {}
    drop(stdin);
    assert!(child.wait().unwrap().success());
    fs::remove_dir_all(parent).unwrap();
}

#[test]
fn stdio_server_does_not_publish_suppressed_diagnostics_as_errors() {
    let root = std::env::temp_dir().join(format!("wae-lsp-suppressed-{}", std::process::id()));
    fs::create_dir_all(root.join("src")).unwrap();
    let root = root.canonicalize().unwrap();
    fs::write(
        root.join("src/a.ts"),
        "// wae-ignore-file ARCH-001 -- approved migration exception\nimport './a';",
    )
    .unwrap();
    fs::write(root.join("wae.yaml"), "version: 1\n").unwrap();
    let source_uri = Url::from_file_path(root.join("src/a.ts")).unwrap().to_string();
    let mut child = Command::new(env!("CARGO_BIN_EXE_wae-lsp"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let stdout = message_receiver(child.stdout.take().unwrap());
    send(
        &mut stdin,
        &json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":Url::from_directory_path(&root).unwrap().to_string()}}),
    );
    assert_eq!(receive(&stdout)["id"], 1);
    send(&mut stdin, &json!({"jsonrpc":"2.0","method":"initialized","params":{}}));
    let diagnostics = loop {
        let message = receive(&stdout);
        if message["method"] == "textDocument/publishDiagnostics"
            && message["params"]["uri"] == source_uri
        {
            break message["params"]["diagnostics"].clone();
        }
    };
    assert_eq!(diagnostics, json!([]));
    send(&mut stdin, &json!({"jsonrpc":"2.0","id":2,"method":"shutdown","params":null}));
    send(&mut stdin, &json!({"jsonrpc":"2.0","method":"exit","params":null}));
    while receive(&stdout)["id"] != 2 {}
    drop(stdin);
    assert!(child.wait().unwrap().success());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn stdio_server_reports_the_shared_synthetic_app_golden() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/synthetic-app")
        .canonicalize()
        .unwrap();
    let golden: Value = serde_json::from_str(
        &fs::read_to_string(fixture.join("expected-diagnostics.json")).unwrap(),
    )
    .unwrap();
    let root_uri = Url::from_directory_path(&fixture).unwrap().to_string();
    let mut child = Command::new(env!("CARGO_BIN_EXE_wae-lsp"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let stdout = message_receiver(child.stdout.take().unwrap());
    send(
        &mut stdin,
        &json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":root_uri}}),
    );
    assert_eq!(receive(&stdout)["id"], 1);
    send(&mut stdin, &json!({"jsonrpc":"2.0","method":"initialized","params":{}}));

    let source_modules = golden["sourceModules"].as_u64().unwrap() as usize;
    let mut published = std::collections::BTreeMap::<String, Vec<Value>>::new();
    while published.len() < source_modules {
        let message = receive(&stdout);
        if message["method"] == "textDocument/publishDiagnostics" {
            // Compare URI text: on Windows `canonicalize` yields a `\\?\C:\...` verbatim path while
            // `Url::to_file_path` yields `C:\...`, so filesystem path prefixes would not match.
            let uri = message["params"]["uri"].as_str().unwrap();
            let relative = uri
                .strip_prefix(root_uri.as_str())
                .unwrap_or_else(|| panic!("{uri} is outside {root_uri}"))
                .to_string();
            published
                .insert(relative, message["params"]["diagnostics"].as_array().unwrap().clone());
        }
    }
    let mut actual = published
        .iter()
        .flat_map(|(file, diagnostics)| {
            diagnostics.iter().map(move |diagnostic| {
                (
                    diagnostic["code"].as_str().unwrap().to_string(),
                    file.clone(),
                    diagnostic["range"]["start"]["line"].as_u64().unwrap() + 1,
                    diagnostic["range"]["start"]["character"].as_u64().unwrap() + 1,
                    diagnostic["data"]["fingerprint"].as_str().unwrap().to_string(),
                )
            })
        })
        .collect::<Vec<_>>();
    let mut expected = golden["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|diagnostic| {
            (
                diagnostic["ruleId"].as_str().unwrap().to_string(),
                diagnostic["file"].as_str().unwrap().to_string(),
                diagnostic["line"].as_u64().unwrap(),
                diagnostic["column"].as_u64().unwrap(),
                diagnostic["fingerprint"].as_str().unwrap().to_string(),
            )
        })
        .collect::<Vec<_>>();
    actual.sort();
    expected.sort();
    assert_eq!(actual, expected, "the editor must show exactly the CLI/MCP golden diagnostics");

    // Transitive runtime violations carry their full path to the editor.
    let runtime = published["apps/web/src/app/profile/profile-card.tsx"]
        .iter()
        .find(|diagnostic| diagnostic["code"] == "RUNTIME-001")
        .unwrap();
    assert_eq!(runtime["data"]["dependencyPath"].as_array().unwrap().len(), 4);
    assert!(
        runtime["message"]
            .as_str()
            .unwrap()
            .contains("Path: apps/web/src/app/profile/profile-card.tsx →")
    );
    assert_eq!(runtime["relatedInformation"].as_array().unwrap().len(), 3);

    send(&mut stdin, &json!({"jsonrpc":"2.0","id":2,"method":"shutdown","params":null}));
    send(&mut stdin, &json!({"jsonrpc":"2.0","method":"exit","params":null}));
    loop {
        if receive(&stdout)["id"] == 2 {
            break;
        }
    }
    drop(stdin);
    assert!(child.wait().unwrap().success());
}
