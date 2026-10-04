use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};

use serde_json::{Value, json};
use url::Url;

fn send(stdin: &mut impl Write, message: &Value) {
    let body = serde_json::to_vec(message).unwrap();
    write!(stdin, "Content-Length: {}\r\n\r\n", body.len()).unwrap();
    stdin.write_all(&body).unwrap();
    stdin.flush().unwrap();
}

fn receive(reader: &mut BufReader<impl Read>) -> Value {
    let mut length = None;
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).unwrap();
        if header == "\r\n" {
            break;
        }
        if let Some(value) = header.strip_prefix("Content-Length:") {
            length = Some(value.trim().parse::<usize>().unwrap());
        }
    }
    let mut body = vec![0; length.expect("LSP Content-Length")];
    reader.read_exact(&mut body).unwrap();
    serde_json::from_slice(&body).unwrap()
}

#[test]
fn stdio_server_publishes_diagnostics_and_shuts_down_cleanly() {
    let root = std::env::temp_dir().join(format!("wae-lsp-e2e-{}", std::process::id()));
    fs::create_dir_all(root.join("src")).unwrap();
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
    let mut stdout = BufReader::new(child.stdout.take().unwrap());

    send(
        &mut stdin,
        &json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":root_uri}}),
    );
    let initialized = receive(&mut stdout);
    assert_eq!(initialized["id"], 1);
    send(&mut stdin, &json!({"jsonrpc":"2.0","method":"initialized","params":{}}));

    let published = loop {
        let message = receive(&mut stdout);
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
    let actions = receive(&mut stdout);
    assert_eq!(actions["id"], 4);
    assert_eq!(actions["result"].as_array().unwrap().len(), 2);
    assert_eq!(
        actions["result"][1]["edit"]["changes"][&document_uri][0]["newText"],
        "// wae-ignore RESOLVE-001 -- \n"
    );
    send(
        &mut stdin,
        &json!({
            "jsonrpc":"2.0", "id":5, "method":"workspace/executeCommand",
            "params":{"command":"wae.showSuggestion","arguments":[{"suggestion":"Resolve the missing module."}]}
        }),
    );
    let suggestion = receive(&mut stdout);
    assert_eq!(suggestion["method"], "window/showMessage");
    assert_eq!(suggestion["params"]["message"], "Resolve the missing module.");
    assert_eq!(receive(&mut stdout)["id"], 5);

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
        let message = receive(&mut stdout);
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
        let message = receive(&mut stdout);
        if message["id"] == 3 {
            break message;
        }
    };
    assert!(hover["result"]["contents"]["value"].as_str().unwrap().contains("WAE architecture"));

    send(&mut stdin, &json!({"jsonrpc":"2.0","id":2,"method":"shutdown","params":null}));
    send(&mut stdin, &json!({"jsonrpc":"2.0","method":"exit","params":null}));
    assert_eq!(receive(&mut stdout)["id"], 2);
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
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
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
    let initialized = receive(&mut stdout);
    assert_eq!(initialized["result"]["capabilities"]["positionEncoding"], "utf-16");
    send(&mut stdin, &json!({"jsonrpc":"2.0","method":"initialized","params":{}}));

    let diagnostic = loop {
        let message = receive(&mut stdout);
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
        let message = receive(&mut stdout);
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
        let message = receive(&mut stdout);
        if message["method"] == "textDocument/publishDiagnostics"
            && message["params"]["uri"] == added_file_uri
            && message["params"]["diagnostics"] == json!([])
        {
            break;
        }
    }

    send(&mut stdin, &json!({"jsonrpc":"2.0","id":2,"method":"shutdown","params":null}));
    send(&mut stdin, &json!({"jsonrpc":"2.0","method":"exit","params":null}));
    while receive(&mut stdout)["id"] != 2 {}
    drop(stdin);
    assert!(child.wait().unwrap().success());
    fs::remove_dir_all(parent).unwrap();
}

#[test]
fn stdio_server_does_not_publish_suppressed_diagnostics_as_errors() {
    let root = std::env::temp_dir().join(format!("wae-lsp-suppressed-{}", std::process::id()));
    fs::create_dir_all(root.join("src")).unwrap();
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
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    send(
        &mut stdin,
        &json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"rootUri":Url::from_directory_path(&root).unwrap().to_string()}}),
    );
    assert_eq!(receive(&mut stdout)["id"], 1);
    send(&mut stdin, &json!({"jsonrpc":"2.0","method":"initialized","params":{}}));
    let diagnostics = loop {
        let message = receive(&mut stdout);
        if message["method"] == "textDocument/publishDiagnostics"
            && message["params"]["uri"] == source_uri
        {
            break message["params"]["diagnostics"].clone();
        }
    };
    assert_eq!(diagnostics, json!([]));
    send(&mut stdin, &json!({"jsonrpc":"2.0","id":2,"method":"shutdown","params":null}));
    send(&mut stdin, &json!({"jsonrpc":"2.0","method":"exit","params":null}));
    while receive(&mut stdout)["id"] != 2 {}
    drop(stdin);
    assert!(child.wait().unwrap().success());
    fs::remove_dir_all(root).unwrap();
}
