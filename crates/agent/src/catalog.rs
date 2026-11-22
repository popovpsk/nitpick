use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::jsonrpc::{self, Options};
use crate::lines::Lines;
use crate::process::{Environment, Grace, Invocation, Process};
use crate::session::driver_for;
use crate::{AcpFlavor, Error, HarnessKind, Launch, acp, claude, codex, pi};

pub const MODELS_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_OUTPUT_BYTES: u64 = 8 * 1024 * 1024;
const MAX_CODEX_PAGES: usize = 20;
const CLAUDE_REQUEST: &str = "init-1";
const PI_REQUEST: &str = "models-1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub is_default: bool,
}

pub async fn models(kind: HarnessKind, command: &str) -> Result<Vec<Model>, Error> {
    models_in(kind, command, None).await
}

pub async fn models_in(
    kind: HarnessKind,
    command: &str,
    env: Option<Environment>,
) -> Result<Vec<Model>, Error> {
    let command = command.to_owned();
    crate::process::on_runtime(async move {
        match fetch(kind, &command, env).await {
            Ok(models) => Ok(models),
            Err(Failure::Error(error)) => Err(error),
            Err(Failure::Text(detail)) => Err(Error::Probe {
                command: command.clone(),
                detail,
            }),
        }
    })
    .await
}

enum Failure {
    Error(Error),
    Text(String),
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        Failure::Error(error)
    }
}

impl From<std::io::Error> for Failure {
    fn from(error: std::io::Error) -> Self {
        Failure::Error(Error::Io(error))
    }
}

impl From<String> for Failure {
    fn from(text: String) -> Self {
        Failure::Text(text)
    }
}

struct Scratch {
    root: tempfile::TempDir,
}

impl Scratch {
    fn create() -> std::io::Result<Scratch> {
        let mut builder = tempfile::Builder::new();
        builder.prefix("nitpick-models-");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            builder.permissions(std::fs::Permissions::from_mode(0o700));
        }
        let root = builder.tempdir()?;
        std::fs::create_dir(root.path().join("cwd"))?;
        std::fs::create_dir(root.path().join("files"))?;
        Ok(Scratch { root })
    }

    fn cwd(&self) -> PathBuf {
        self.root.path().join("cwd")
    }

    fn files(&self) -> PathBuf {
        self.root.path().join("files")
    }
}

fn timed_out() -> Failure {
    Failure::Text(format!("no answer within {MODELS_TIMEOUT:?}"))
}

const LEAVE: Grace = Grace {
    stdin: Duration::from_secs(1),
    term: Duration::from_secs(1),
};

async fn fetch(
    kind: HarnessKind,
    command: &str,
    env: Option<Environment>,
) -> Result<Vec<Model>, Failure> {
    let scratch = Scratch::create()?;
    let env = match env {
        Some(env) => env,
        None => net_runtime::unblock(Environment::current).await,
    };
    if kind == HarnessKind::Acp(AcpFlavor::OpenCode) {
        return opencode_cli(command, env, &scratch.cwd()).await;
    }
    let launch = Launch {
        kind,
        command: command.to_owned(),
        cwd: scratch.cwd(),
        model: None,
        resume: None,
        instructions: String::new(),
        scratch: scratch.files(),
    };
    let driver = driver_for(kind)?;
    let plan = driver.plan(&launch)?;
    let invocation = Invocation::resolve_in(command, plan.args, &plan.env, env, &launch.cwd)?;
    let (mut process, pipes) = Process::spawn(&invocation)?;
    let question = async {
        match kind {
            HarnessKind::Claude => claude_models(pipes.stdout, pipes.stdin).await,
            HarnessKind::Codex => codex_models(pipes.stdout, pipes.stdin).await,
            HarnessKind::Pi => pi_models(pipes.stdout, pipes.stdin).await,
            HarnessKind::Acp(_) => acp_models(pipes.stdout, pipes.stdin, &launch).await,
        }
    };
    let asked = tokio::time::timeout(MODELS_TIMEOUT, question).await;
    let grace = if asked.is_ok() {
        LEAVE
    } else {
        Grace::IMMEDIATE
    };
    let exit = process.shutdown(grace).await;
    match asked {
        Err(_) => Err(timed_out()),
        Ok(Ok(models)) => Ok(models),
        Ok(Err(text)) if text.is_empty() && !exit.stderr.is_empty() => Err(exit.stderr.into()),
        Ok(Err(text)) => Err(text.into()),
    }
}

async fn opencode_cli(command: &str, env: Environment, cwd: &Path) -> Result<Vec<Model>, Failure> {
    let mut invocation = Invocation::resolve_in(command, Vec::new(), &[], env, cwd)?;
    if invocation.args.last().is_some_and(|last| last == "acp") {
        invocation.args.pop();
    }
    invocation.args.push("models".into());
    invocation.args.push("--verbose".into());
    let (mut process, pipes) = Process::spawn(&invocation)?;
    drop(pipes.stdin);
    let mut bytes = Vec::new();
    let read = tokio::time::timeout(
        MODELS_TIMEOUT,
        pipes.stdout.take(MAX_OUTPUT_BYTES).read_to_end(&mut bytes),
    )
    .await;
    let grace = if read.is_ok() {
        LEAVE
    } else {
        Grace::IMMEDIATE
    };
    let exit = process.shutdown(grace).await;
    let Ok(read) = read else {
        return Err(timed_out());
    };
    read?;
    if !exit.status.is_some_and(|status| status.success()) {
        let line = exit.stderr.lines().find(|line| !line.trim().is_empty());
        return Err(line.unwrap_or("it did not finish").to_owned().into());
    }
    Ok(opencode_models(&String::from_utf8_lossy(&bytes)))
}

pub async fn claude_models<R, W>(reader: R, mut writer: W) -> Result<Vec<Model>, String>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let request = claude::wire::initialize_request(CLAUDE_REQUEST);
    write_line(&mut writer, &request).await?;
    let mut lines = Lines::new(reader);
    loop {
        let line = lines
            .next_line()
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "it exited before answering".to_owned())?;
        let Ok(frame) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if frame.get("type").and_then(Value::as_str) != Some("control_response") {
            continue;
        }
        let response = &frame["response"];
        if response.get("request_id").and_then(Value::as_str) != Some(CLAUDE_REQUEST) {
            continue;
        }
        if response.get("subtype").and_then(Value::as_str) != Some("success") {
            return Err(response
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("the request was refused")
                .to_owned());
        }
        return Ok(parse_claude(&response["response"]));
    }
}

pub async fn acp_models<R, W>(reader: R, writer: W, launch: &Launch) -> Result<Vec<Model>, String>
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    let connection = jsonrpc::connect(reader, writer, Options::default());
    let client = connection.client();
    client
        .request("initialize", acp::wire::initialize_params())
        .await
        .map_err(|error| error.to_string())?;
    let created = client
        .request("session/new", acp::wire::new_session_params(launch, false))
        .await
        .map_err(|error| error.to_string())?;
    Ok(parse_acp(&created))
}

pub async fn codex_models<R, W>(reader: R, writer: W) -> Result<Vec<Model>, String>
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    let connection = jsonrpc::connect(reader, writer, Options { tag_version: false });
    let client = connection.client();
    client
        .request("initialize", codex::wire::initialize_params())
        .await
        .map_err(|error| error.to_string())?;
    client
        .notify("initialized", json!({}))
        .map_err(|error| error.to_string())?;
    let mut models = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_CODEX_PAGES {
        let params = match &cursor {
            Some(cursor) => json!({"cursor": cursor}),
            None => json!({}),
        };
        let page = client
            .request("model/list", params)
            .await
            .map_err(|error| error.to_string())?;
        let (found, next) = parse_codex_page(&page);
        models.extend(found);
        match next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    Ok(models)
}

pub async fn pi_models<R, W>(reader: R, mut writer: W) -> Result<Vec<Model>, String>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    write_line(&mut writer, &pi::wire::get_available_models(PI_REQUEST)).await?;
    let mut lines = Lines::new(reader);
    loop {
        let line = lines
            .next_line()
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "it exited before answering".to_owned())?;
        let Ok(frame) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let answers = frame.get("type").and_then(Value::as_str) == Some("response")
            && frame.get("command").and_then(Value::as_str) == Some("get_available_models");
        if !answers {
            continue;
        }
        if frame.get("success").and_then(Value::as_bool) != Some(true) {
            return Err(frame
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("the request failed")
                .to_owned());
        }
        return Ok(parse_pi(&frame["data"]));
    }
}

async fn write_line<W: AsyncWrite + Unpin>(writer: &mut W, line: &str) -> Result<(), String> {
    let mut bytes = Vec::with_capacity(line.len() + 1);
    bytes.extend_from_slice(line.as_bytes());
    bytes.push(b'\n');
    writer
        .write_all(&bytes)
        .await
        .map_err(|error| error.to_string())?;
    writer.flush().await.map_err(|error| error.to_string())
}

fn text(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

pub fn parse_claude(response: &Value) -> Vec<Model> {
    let entries = response
        .get("models")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    let default = entries
        .iter()
        .find(|entry| text(entry, "value").as_deref() == Some("default"))
        .and_then(|entry| text(entry, "resolvedModel"));
    let mut models: Vec<Model> = Vec::new();
    for entry in entries {
        let Some(id) = text(entry, "value") else {
            continue;
        };
        if id == "default" {
            continue;
        }
        let resolved = text(entry, "resolvedModel");
        let is_default = default.is_some() && resolved == default;
        models.push(Model {
            name: text(entry, "displayName").unwrap_or_else(|| id.clone()),
            description: text(entry, "description"),
            is_default: is_default && !models.iter().any(|model| model.is_default),
            id,
        });
    }
    models
}

pub fn opencode_models(output: &str) -> Vec<Model> {
    let mut models = Vec::new();
    let mut block: Option<String> = None;
    for line in output.lines() {
        match &mut block {
            Some(json) => {
                json.push_str(line);
                json.push('\n');
                if line == "}"
                    && let Some(model) = block.take().and_then(|json| opencode_block(&json))
                {
                    models.push(model);
                }
            }
            None if line == "{" => block = Some("{\n".to_owned()),
            None => {}
        }
    }
    if models.is_empty() {
        for line in output.lines().map(str::trim) {
            if line.contains('/') && !line.contains(char::is_whitespace) {
                models.push(Model {
                    id: line.to_owned(),
                    name: line.to_owned(),
                    description: None,
                    is_default: false,
                });
            }
        }
    }
    models
}

fn opencode_block(json: &str) -> Option<Model> {
    let value: Value = serde_json::from_str(json).ok()?;
    let id = text(&value, "id")?;
    let provider = text(&value, "providerID")?;
    let full = format!("{provider}/{id}");
    Some(Model {
        name: text(&value, "name").unwrap_or_else(|| full.clone()),
        id: full,
        description: Some(provider),
        is_default: false,
    })
}

pub fn parse_acp(result: &Value) -> Vec<Model> {
    let mut models = Vec::new();
    if let Some(options) = result.get("configOptions").and_then(Value::as_array)
        && let Some(option) = options
            .iter()
            .find(|option| option.get("category").and_then(Value::as_str) == Some("model"))
    {
        let current = text(option, "currentValue");
        collect_options(option.get("options"), current.as_deref(), &mut models);
    }
    if models.is_empty()
        && let Some(available) = result.get("models")
    {
        let current = text(available, "currentModelId");
        for entry in available
            .get("availableModels")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(id) = text(entry, "modelId") else {
                continue;
            };
            models.push(Model {
                name: text(entry, "name").unwrap_or_else(|| id.clone()),
                description: text(entry, "description"),
                is_default: current.as_deref() == Some(id.as_str()),
                id,
            });
        }
    }
    models
}

fn collect_options(options: Option<&Value>, current: Option<&str>, out: &mut Vec<Model>) {
    for entry in options.and_then(Value::as_array).into_iter().flatten() {
        match text(entry, "value") {
            Some(id) => out.push(Model {
                name: text(entry, "name").unwrap_or_else(|| id.clone()),
                description: text(entry, "description"),
                is_default: current == Some(id.as_str()),
                id,
            }),
            None => collect_options(entry.get("options"), current, out),
        }
    }
}

pub fn parse_codex_page(result: &Value) -> (Vec<Model>, Option<String>) {
    let models = result
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|entry| entry.get("hidden").and_then(Value::as_bool) != Some(true))
        .filter_map(|entry| {
            let id = text(entry, "model").or_else(|| text(entry, "id"))?;
            Some(Model {
                name: text(entry, "displayName").unwrap_or_else(|| id.clone()),
                description: text(entry, "description"),
                is_default: entry.get("isDefault").and_then(Value::as_bool) == Some(true),
                id,
            })
        })
        .collect();
    (models, text(result, "nextCursor"))
}

pub fn parse_pi(data: &Value) -> Vec<Model> {
    data.get("models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let id = text(entry, "id")?;
            let provider = text(entry, "provider");
            let full = match &provider {
                Some(provider) => format!("{provider}/{id}"),
                None => id.clone(),
            };
            Some(Model {
                name: text(entry, "name").unwrap_or(id),
                id: full,
                description: provider,
                is_default: false,
            })
        })
        .collect()
}
