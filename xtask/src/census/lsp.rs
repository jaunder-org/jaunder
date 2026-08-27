//! Language Server Protocol transport and response decoding.

use super::{CollectorContext, Language};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

pub(crate) fn analyzer_stdio_args(language: Language) -> &'static [&'static str] {
    match language {
        Language::Rust => &[],
        Language::TypeScript => &["--stdio"],
        Language::Elisp | Language::Repository => &[],
    }
}

pub(crate) struct LspClient {
    child: std::process::Child,
    input: std::process::ChildStdin,
    stderr: std::process::ChildStderr,
    responses: Receiver<Result<serde_json::Value, String>>,
    next_id: u64,
}

impl Drop for LspClient {
    fn drop(&mut self) {
        match self.child.try_wait() {
            Ok(Some(_)) => {}
            Ok(None) => {
                if let Err(error) = self.child.kill() {
                    eprintln!("warning: stopping census LSP server failed: {error}");
                }
                if let Err(error) = self.child.wait() {
                    eprintln!("warning: reaping census LSP server failed: {error}");
                }
            }
            Err(error) => {
                eprintln!("warning: checking census LSP server status failed: {error}");
                if let Err(error) = self.child.wait() {
                    eprintln!(
                        "warning: reaping census LSP server after status failure failed: {error}"
                    );
                }
            }
        }
        let mut stderr = String::new();
        if let Err(error) = self.stderr.read_to_string(&mut stderr) {
            eprintln!("warning: reading census LSP server stderr during cleanup failed: {error}");
        } else if !stderr.trim().is_empty() {
            eprintln!(
                "warning: census LSP server stderr during cleanup: {}",
                stderr.trim()
            );
        }
    }
}
impl LspClient {
    pub(crate) fn start(
        context: &CollectorContext,
        language: Language,
        analyzer: &str,
    ) -> Result<Self, String> {
        let mut command = Command::new(analyzer);
        command
            .args(analyzer_stdio_args(language))
            .current_dir(&context.repo_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|error| format!("starting `{analyzer}` LSP server: {error}"))?;
        let input = child
            .stdin
            .take()
            .ok_or_else(|| format!("`{analyzer}` did not provide LSP stdin"))?;
        let output = child
            .stdout
            .take()
            .ok_or_else(|| format!("`{analyzer}` did not provide LSP stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| format!("`{analyzer}` did not provide LSP stderr"))?;
        let (sender, responses) = mpsc::sync_channel(16);
        std::thread::spawn(move || {
            let mut output = BufReader::new(output);
            loop {
                let result = read_lsp_message(&mut output);
                let done = result.is_err();
                if sender.send(result).is_err() || done {
                    break;
                }
            }
        });
        let mut client = Self {
            child,
            input,
            stderr,
            responses,
            next_id: 1,
        };
        let root = absolute_repo_root(context)?;
        let root_uri = url::Url::from_directory_path(&root)
            .map_err(|_| format!("cannot form LSP root URI for {}", root.display()))?
            .to_string();
        client.request(
            "initialize",
            serde_json::json!({
                "processId": null,
                "rootUri": root_uri.clone(),
                "workspaceFolders": [{ "uri": root_uri, "name": "repository" }],
                "capabilities": {
                    "workspace": { "symbol": {}, "workspaceFolders": true },
                    "textDocument": { "references": {} }
                }
            }),
        )?;
        client.notify("initialized", serde_json::json!({}))?;
        Ok(client)
    }

    pub(crate) fn open_documents(
        &mut self,
        context: &CollectorContext,
        language: Language,
    ) -> Result<(), String> {
        for params in lsp_open_document_params(context, language)? {
            self.notify("textDocument/didOpen", params)?;
        }
        Ok(())
    }

    fn lsp_language_id(language: Language, path: &str) -> Option<&'static str> {
        language.lsp_language_id(path)
    }

    fn request(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(
            serde_json::json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
        )?;
        loop {
            let message = self.read()?;
            if message.get("method").is_some() && message.get("id").is_some() {
                self.send(serde_json::json!({ "jsonrpc": "2.0", "id": message["id"].clone(), "result": null }))?;
                continue;
            }
            if message.get("id").and_then(serde_json::Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = message.get("error") {
                return Err(format!("LSP `{method}` failed: {error}"));
            }
            return message
                .get("result")
                .cloned()
                .ok_or_else(|| format!("LSP `{method}` response omitted result"));
        }
    }

    fn notify(&mut self, method: &str, params: serde_json::Value) -> Result<(), String> {
        self.send(serde_json::json!({ "jsonrpc": "2.0", "method": method, "params": params }))
    }

    fn send(&mut self, message: serde_json::Value) -> Result<(), String> {
        let bytes = serde_json::to_vec(&message)
            .map_err(|error| format!("serializing LSP request: {error}"))?;
        write!(self.input, "Content-Length: {}\r\n\r\n", bytes.len())
            .map_err(|error| format!("writing LSP header: {error}"))?;
        self.input
            .write_all(&bytes)
            .map_err(|error| format!("writing LSP body: {error}"))?;
        self.input
            .flush()
            .map_err(|error| format!("flushing LSP request: {error}"))
    }

    fn read(&mut self) -> Result<serde_json::Value, String> {
        self.responses
            .recv_timeout(Duration::from_secs(30))
            .map_err(|error| format!("waiting for LSP response: {error}"))?
    }
}

pub(crate) fn lsp_open_document_params(
    context: &CollectorContext,
    language: Language,
) -> Result<Vec<serde_json::Value>, String> {
    let root = absolute_repo_root(context)?;
    context
        .snapshot
        .files
        .iter()
        .filter_map(|file| {
            LspClient::lsp_language_id(language, &file.path).map(|language_id| (file, language_id))
        })
        .map(|(file, language_id)| {
            let uri = url::Url::from_file_path(root.join(&file.path))
                .map_err(|_| format!("cannot form LSP document URI for {}", file.path))?
                .to_string();
            Ok(serde_json::json!({
                "textDocument": {
                    "uri": uri,
                    "languageId": language_id,
                    "version": 1,
                    "text": file.content,
                }
            }))
        })
        .collect()
}

fn read_lsp_message(
    output: &mut BufReader<std::process::ChildStdout>,
) -> Result<serde_json::Value, String> {
    let mut length = None;
    loop {
        let mut line = String::new();
        let read = output
            .read_line(&mut line)
            .map_err(|error| format!("reading LSP header: {error}"))?;
        if read == 0 {
            return Err("LSP server closed stdout".into());
        }

        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(value) = line.strip_prefix("Content-Length:") {
            length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|error| format!("invalid LSP content length: {error}"))?,
            );
        }
    }
    let length = length.ok_or_else(|| "LSP response omitted Content-Length".to_owned())?;
    let mut body = vec![0; length];
    output
        .read_exact(&mut body)
        .map_err(|error| format!("reading LSP response body: {error}"))?;
    serde_json::from_slice(&body).map_err(|error| format!("malformed LSP JSON response: {error}"))
}
impl super::semantic::SemanticSession for LspClient {
    fn request(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        LspClient::request(self, method, params)
    }
}

pub(crate) struct SemanticSymbol {
    pub(crate) name: String,
    pub(crate) uri: String,
    pub(crate) line: u32,
    pub(crate) character: u32,
    pub(crate) path: String,
}

pub(crate) fn symbols_from_response(
    context: &CollectorContext,
    response: serde_json::Value,
) -> Result<Option<Vec<SemanticSymbol>>, String> {
    if response.is_null() {
        return Ok(None);
    }
    let entries = response
        .as_array()
        .ok_or_else(|| "LSP workspace/symbol result is not an array".to_owned())?;
    let mut symbols = Vec::with_capacity(entries.len());
    for entry in entries {
        let name = entry
            .get("name")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| "LSP workspace/symbol entry omitted name".to_owned())?
            .to_owned();
        let location = entry
            .get("location")
            .ok_or_else(|| "LSP workspace/symbol entry omitted location".to_owned())?;
        let uri = location
            .get("uri")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| "LSP workspace/symbol location omitted URI".to_owned())?
            .to_owned();
        let start = location
            .get("range")
            .and_then(|range| range.get("start"))
            .ok_or_else(|| "LSP workspace/symbol location omitted range start".to_owned())?;
        let line = start
            .get("line")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| "LSP workspace/symbol range start omitted line".to_owned())?
            as u32;
        let character = start
            .get("character")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| "LSP workspace/symbol range start omitted character".to_owned())?
            as u32;
        let path = uri_to_relative(context, &uri)
            .ok_or_else(|| format!("LSP symbol URI is outside repository: {uri}"))?;
        symbols.push(SemanticSymbol {
            name,
            uri,
            line,
            character,
            path,
        });
    }
    Ok(Some(symbols))
}
pub(crate) fn hover_reports_exported(language: Language, response: serde_json::Value) -> bool {
    let text = response
        .get("contents")
        .and_then(|contents| {
            contents
                .get("value")
                .and_then(serde_json::Value::as_str)
                .or_else(|| contents.as_str())
        })
        .unwrap_or_default();
    match language {
        Language::Rust => text.contains("pub "),
        Language::TypeScript => text.contains("export "),
        Language::Elisp | Language::Repository => false,
    }
}

pub(crate) fn references_from_response(
    context: &CollectorContext,
    response: serde_json::Value,
) -> Result<Vec<String>, String> {
    if response.is_null() {
        return Ok(Vec::new());
    }
    let entries = response
        .as_array()
        .ok_or_else(|| "LSP textDocument/references result is not an array".to_owned())?;
    entries
        .iter()
        .map(|entry| {
            let uri = entry
                .get("uri")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "LSP reference entry omitted URI".to_owned())?;
            uri_to_relative(context, uri)
                .ok_or_else(|| format!("LSP reference URI is outside repository: {uri}"))
        })
        .collect()
}
fn absolute_repo_root(context: &CollectorContext) -> Result<std::path::PathBuf, String> {
    if context.repo_root.is_absolute() {
        Ok(context.repo_root.clone())
    } else {
        std::env::current_dir()
            .map(|current| current.join(&context.repo_root))
            .map_err(|error| format!("resolving census repository root: {error}"))
    }
}
fn uri_to_relative(context: &CollectorContext, uri: &str) -> Option<String> {
    let path = url::Url::parse(uri).ok()?.to_file_path().ok()?;
    let root = absolute_repo_root(context).ok()?;
    Some(path.strip_prefix(root).ok()?.to_str()?.replace("\\", "/"))
}
