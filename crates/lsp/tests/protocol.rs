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

    let document_uri = Url::from_file_path(root.join("src/a.ts")).unwrap().to_string();
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
