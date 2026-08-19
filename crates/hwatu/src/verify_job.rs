// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
//! Harness-neutral UI verification jobs.
//!
//! `hwatu verify <spec.json>` owns the repetitive local orchestration around
//! the daemon's fast `check` primitive: an optional preflight command, optional
//! dev-server lifecycle, readiness polling, a responsive screenshot sweep,
//! deterministic page assertions, source staleness detection, and one evidence
//! report. The same executor backs the MCP `verify_ui` tool, so callers differ
//! only in transport, not verification semantics.

use hwatu_ipc::{LoadStage, Request, Response, Viewport};
use serde_json::{json, Value};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const SCHEMA_VERSION: u64 = 1;
const DEFAULT_READY_TIMEOUT_MS: u64 = 15_000;
const DEFAULT_COMMAND_TIMEOUT_MS: u64 = 120_000;
const POLL_MS: u64 = 50;
static CANCELLATION_REQUESTED: AtomicBool = AtomicBool::new(false);

const DEFAULT_ASSERTION_JS: &str = r#"
const root = document.documentElement;
const body = document.body;
const scrollWidth = Math.max(root?.scrollWidth ?? 0, body?.scrollWidth ?? 0);
const viewportWidth = window.innerWidth;
return {
  ok: document.readyState === 'interactive' || document.readyState === 'complete',
  ready_state: document.readyState,
  viewport: { width: viewportWidth, height: window.innerHeight, dpr: window.devicePixelRatio },
  horizontal_overflow: scrollWidth > viewportWidth + 1,
  overflow_px: Math.max(0, scrollWidth - viewportWidth),
  body_text_chars: (body?.innerText ?? '').length
};
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tier {
    Micro,
    Layout,
    Interactive,
}

impl Tier {
    fn parse(value: Option<&str>) -> Result<Self, String> {
        match value.unwrap_or("micro") {
            "micro" => Ok(Self::Micro),
            "layout" => Ok(Self::Layout),
            "interactive" => Ok(Self::Interactive),
            other => Err(format!(
                "invalid tier {other:?}; want micro|layout|interactive"
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Micro => "micro",
            Self::Layout => "layout",
            Self::Interactive => "interactive",
        }
    }

    fn default_full(self) -> bool {
        !matches!(self, Self::Micro)
    }
}

#[derive(Debug, Clone)]
struct CommandSpec {
    argv: Vec<String>,
    timeout_ms: u64,
}

#[derive(Debug, Clone)]
struct VerifySpec {
    name: String,
    cwd: PathBuf,
    url: String,
    tier: Tier,
    preflight: Option<CommandSpec>,
    server: Option<CommandSpec>,
    ready_timeout_ms: u64,
    viewports: Vec<Viewport>,
    assertion_js: String,
    full: bool,
    fail_on_console: bool,
    source_files: Vec<PathBuf>,
    source_root: Option<PathBuf>,
    artifacts_dir: PathBuf,
    report_path: PathBuf,
    timeout_ms: Option<u64>,
}

impl VerifySpec {
    fn parse(mut value: Value, base: &Path) -> Result<Self, String> {
        let object = value
            .as_object_mut()
            .ok_or("verify spec must be a JSON object")?;
        for key in object.keys() {
            if !matches!(
                key.as_str(),
                "version"
                    | "name"
                    | "cwd"
                    | "url"
                    | "tier"
                    | "preflight"
                    | "server"
                    | "ready_timeout_ms"
                    | "viewports"
                    | "assertion_js"
                    | "full"
                    | "fail_on_console"
                    | "source_files"
                    | "artifacts_dir"
                    | "report_path"
                    | "timeout_ms"
            ) {
                return Err(format!("unknown verify spec field `{key}`"));
            }
        }
        let version = object
            .get("version")
            .and_then(Value::as_u64)
            .unwrap_or(SCHEMA_VERSION);
        if version != SCHEMA_VERSION {
            return Err(format!(
                "unsupported verify spec version {version}; want {SCHEMA_VERSION}"
            ));
        }
        let name = string_field(object.get("name"), "name")?.to_string();
        validate_job_name(&name)?;
        let url = string_field(object.get("url"), "url")?.to_string();
        let tier = Tier::parse(object.get("tier").and_then(Value::as_str))?;

        let cwd = object
            .get("cwd")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .unwrap_or_else(|| base.to_path_buf());
        let cwd = absolutize(base, &cwd);

        let preflight = parse_command(object.get("preflight"), "preflight")?;
        let server = parse_command(object.get("server"), "server")?;
        let ready_timeout_ms = object
            .get("ready_timeout_ms")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_READY_TIMEOUT_MS);
        let viewports = parse_viewports(object.get("viewports"))?;
        let custom_assertion = object.get("assertion_js").and_then(Value::as_str);
        let assertion_js = compose_assertion(custom_assertion);
        if tier == Tier::Interactive && object.get("assertion_js").is_none() {
            return Err("interactive verify jobs require `assertion_js`".into());
        }
        let full = object
            .get("full")
            .and_then(Value::as_bool)
            .unwrap_or_else(|| tier.default_full());
        let fail_on_console = object
            .get("fail_on_console")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let source_files = parse_string_array(object.get("source_files"), "source_files")?
            .into_iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        if source_files.is_empty() {
            return Err("verify spec needs at least one `source_files` entry".into());
        }

        let artifacts_dir = object
            .get("artifacts_dir")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(format!(".hwatu/verify/{name}")));
        let artifacts_dir = absolutize(&cwd, &artifacts_dir);
        let report_path = object
            .get("report_path")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .unwrap_or_else(|| artifacts_dir.join("report.json"));
        let report_path = absolutize(&cwd, &report_path);
        let timeout_ms = object.get("timeout_ms").and_then(Value::as_u64);

        Ok(Self {
            name,
            cwd,
            url,
            tier,
            preflight,
            server,
            ready_timeout_ms,
            viewports,
            assertion_js,
            full,
            fail_on_console,
            source_files,
            source_root: None,
            artifacts_dir,
            report_path,
            timeout_ms,
        })
    }
}

fn compose_assertion(custom: Option<&str>) -> String {
    let Some(custom) = custom else {
        return DEFAULT_ASSERTION_JS.to_string();
    };
    format!(
        "const __core = await (async () => {{ {DEFAULT_ASSERTION_JS} }})();\n\
         const __custom = await (async () => {{ {custom} }})();\n\
         return {{ ...__core, assertion: __custom, ok: __core.ok && \
         (__custom === true || __custom?.ok === true) }};"
    )
}

fn string_field<'a>(value: Option<&'a Value>, name: &str) -> Result<&'a str, String> {
    value
        .and_then(Value::as_str)
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| format!("verify spec needs non-empty string `{name}`"))
}

fn validate_job_name(name: &str) -> Result<(), String> {
    if name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        Ok(())
    } else {
        Err("verify job `name` may contain only letters, digits, '-' and '_'".into())
    }
}

fn parse_command(value: Option<&Value>, name: &str) -> Result<Option<CommandSpec>, String> {
    let Some(value) = value else { return Ok(None) };
    if value.is_null() {
        return Ok(None);
    }
    let object = value
        .as_object()
        .ok_or_else(|| format!("`{name}` must be an object"))?;
    if let Some(key) = object
        .keys()
        .find(|key| !matches!(key.as_str(), "argv" | "timeout_ms"))
    {
        return Err(format!("unknown `{name}` field `{key}`"));
    }
    let argv = parse_string_array(object.get("argv"), &format!("{name}.argv"))?;
    if argv.is_empty() {
        return Err(format!("`{name}.argv` must not be empty"));
    }
    let timeout_ms = object
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_COMMAND_TIMEOUT_MS);
    Ok(Some(CommandSpec { argv, timeout_ms }))
}

fn parse_string_array(value: Option<&Value>, name: &str) -> Result<Vec<String>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let array = value
        .as_array()
        .ok_or_else(|| format!("`{name}` must be an array of strings"))?;
    array
        .iter()
        .map(|entry| {
            entry
                .as_str()
                .filter(|v| !v.is_empty())
                .map(str::to_string)
                .ok_or_else(|| format!("`{name}` must contain only non-empty strings"))
        })
        .collect()
}

fn parse_viewports(value: Option<&Value>) -> Result<Vec<Viewport>, String> {
    let Some(value) = value else {
        return Viewport::parse_list("390x844,768x1024,1440x1000");
    };
    if let Some(list) = value.as_str() {
        return Viewport::parse_list(list);
    }
    let entries = value
        .as_array()
        .ok_or("`viewports` must be a comma-separated string or string array")?;
    let mut joined = Vec::with_capacity(entries.len());
    for entry in entries {
        joined.push(
            entry
                .as_str()
                .ok_or("`viewports` array must contain strings")?,
        );
    }
    Viewport::parse_list(&joined.join(","))
}

fn absolutize(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

/// CLI entry point. The one positional argument is a JSON spec, or use
/// `--stdin` to read the same object from stdin.
pub fn run_cli(args: &[String]) -> i32 {
    match load_cli_spec(args).and_then(execute) {
        Ok(report) => {
            println!(
                "{}",
                serde_json::to_string(&report).expect("serialize report")
            );
            if report.get("passed").and_then(Value::as_bool) == Some(true) {
                0
            } else {
                1
            }
        }
        Err(error) => {
            eprintln!("hwatu verify: {error}");
            2
        }
    }
}

/// MCP entry point. `spec_path` and inline `spec` are deliberately equivalent;
/// both feed the exact same parser and executor as the CLI.
pub fn run_mcp(args: &Value) -> Result<String, String> {
    let (spec, inline) = match (
        args.get("spec_path").and_then(Value::as_str),
        args.get("spec"),
    ) {
        (Some(path), None) => (load_spec_file(Path::new(path))?, false),
        (None, Some(value)) => {
            let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
            let (source_root, source_files) = validate_inline_spec(value, &cwd)?;
            let mut spec = VerifySpec::parse(value.clone(), &cwd)?;
            spec.source_root = Some(source_root);
            spec.source_files = source_files;
            spec.artifacts_dir = inline_artifacts_dir(&spec.name);
            spec.report_path = spec.artifacts_dir.join("report.json");
            (spec, true)
        }
        (Some(_), Some(_)) => return Err("verify_ui takes `spec_path` or `spec`, not both".into()),
        (None, None) => return Err("verify_ui needs `spec_path` or inline `spec`".into()),
    };
    if inline && (spec.preflight.is_some() || spec.server.is_some()) {
        return Err(
            "inline MCP verify specs cannot execute commands; put preflight/server argv in a reviewed spec file and pass `spec_path`"
                .into(),
        );
    }
    let report = execute(spec)?;
    serialize_mcp_report(&report)
}

fn validate_inline_spec(value: &Value, cwd: &Path) -> Result<(PathBuf, Vec<PathBuf>), String> {
    let object = value
        .as_object()
        .ok_or("inline verify spec must be a JSON object")?;
    for field in ["preflight", "server", "cwd", "artifacts_dir", "report_path"] {
        if object.contains_key(field) {
            return Err(format!(
                "inline MCP verify specs cannot set `{field}`; use a reviewed spec_path"
            ));
        }
    }
    let root = fs::canonicalize(cwd)
        .map_err(|e| format!("resolve MCP working directory {}: {e}", cwd.display()))?;
    let mut resolved_sources = Vec::new();
    for source in parse_string_array(object.get("source_files"), "source_files")? {
        let path = Path::new(&source);
        if path.is_absolute()
            || path.components().any(|part| {
                matches!(
                    part,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(
                "inline MCP verify source_files must stay inside the MCP working directory".into(),
            );
        }
        let resolved = fs::canonicalize(cwd.join(path))
            .map_err(|e| format!("resolve inline source file {source}: {e}"))?;
        if !resolved.starts_with(&root) {
            return Err(
                "inline MCP verify source_files cannot escape through workspace symlinks".into(),
            );
        }
        // Retain the canonical path that was validated. Reading later also
        // verifies the opened file descriptor, closing symlink-swap TOCTOU.
        resolved_sources.push(resolved);
    }
    Ok((root, resolved_sources))
}

fn inline_artifacts_dir(name: &str) -> PathBuf {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("hwatu-{}", std::process::id())));
    runtime.join("hwatu").join("verify-inline").join(name)
}

fn serialize_mcp_report(report: &Value) -> Result<String, String> {
    let text = serde_json::to_string(report).map_err(|e| e.to_string())?;
    if report.get("passed").and_then(Value::as_bool) == Some(true) {
        Ok(text)
    } else {
        Err(text)
    }
}

fn load_cli_spec(args: &[String]) -> Result<VerifySpec, String> {
    match args {
        [flag] if flag == "--stdin" => {
            let mut input = String::new();
            std::io::stdin()
                .read_to_string(&mut input)
                .map_err(|e| format!("read stdin: {e}"))?;
            let value =
                serde_json::from_str(&input).map_err(|e| format!("parse stdin JSON: {e}"))?;
            let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
            VerifySpec::parse(value, &cwd)
        }
        [path] => load_spec_file(Path::new(path)),
        _ => Err("usage: hwatu verify <spec.json> | hwatu verify --stdin".into()),
    }
}

fn load_spec_file(path: &Path) -> Result<VerifySpec, String> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path)
    };
    let input = fs::read_to_string(&path)
        .map_err(|e| format!("read verify spec {}: {e}", path.display()))?;
    let value = serde_json::from_str(&input)
        .map_err(|e| format!("parse verify spec {}: {e}", path.display()))?;
    let base = path.parent().unwrap_or(Path::new("."));
    VerifySpec::parse(value, base)
}

fn execute(spec: VerifySpec) -> Result<Value, String> {
    fs::create_dir_all(&spec.artifacts_dir)
        .map_err(|e| format!("create artifacts dir {}: {e}", spec.artifacts_dir.display()))?;
    if let Some(parent) = spec.report_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("create report dir {}: {e}", parent.display()))?;
    }

    let _lease = Lease::acquire(&spec)?;
    let _signals = CancellationSignals::install()?;
    let started = Instant::now();
    let spec_fingerprint = spec_fingerprint(&spec);
    let source_before = source_fingerprint(&spec)?;
    let mut findings = Vec::<String>::new();

    let preflight = match &spec.preflight {
        Some(command) => match run_command(
            command,
            &spec.cwd,
            &spec.artifacts_dir.join("preflight.log"),
        ) {
            Ok(report) => {
                if !report.get("ok").and_then(Value::as_bool).unwrap_or(false) {
                    findings.push("preflight command failed".into());
                }
                Some(report)
            }
            Err(error) => {
                findings.push(format!("preflight command could not run: {error}"));
                Some(json!({ "ok": false, "error": error }))
            }
        },
        None => None,
    };

    let (server_report, check) = if findings.is_empty() {
        match ServerGuard::prepare(&spec) {
            Ok(mut server) => {
                let server_report = server.report();
                let check = match dispatch_check(&spec) {
                    Ok(check) => check,
                    Err(error) => {
                        findings.push(error.clone());
                        json!({ "error": error })
                    }
                };
                server.stop();
                (server_report, check)
            }
            Err(error) => {
                findings.push(error.clone());
                (json!({ "ready": false, "error": error }), Value::Null)
            }
        }
    } else {
        (
            json!({ "skipped": true, "reason": "preflight failed" }),
            Value::Null,
        )
    };
    if !check.is_null() {
        inspect_check(&check, &spec, &mut findings);
    }
    let artifact_fingerprints = artifact_fingerprints(&check);

    let source_after = match source_fingerprint(&spec) {
        Ok(hash) => {
            if source_before != hash {
                findings.push("source files changed during verification; evidence is stale".into());
            }
            Some(hash)
        }
        Err(error) => {
            findings.push(format!(
                "source files became unreadable during verification; evidence is stale: {error}"
            ));
            None
        }
    };

    let passed = findings.is_empty();
    let report = json!({
        "schema_version": SCHEMA_VERSION,
        "job": spec.name,
        "tier": spec.tier.as_str(),
        "passed": passed,
        "status": if passed { "passed" } else { "failed" },
        "url": spec.url,
        "cwd": spec.cwd,
        "spec_fingerprint": spec_fingerprint,
        "source_fingerprint": source_before,
        "source_fingerprint_after": source_after,
        "source_files": spec.source_files,
        "preflight": preflight,
        "server": server_report,
        "check": check,
        "artifact_fingerprints": artifact_fingerprints,
        "findings": findings,
        "artifacts_dir": spec.artifacts_dir,
        "report_path": spec.report_path,
        "total_ms": started.elapsed().as_millis() as u64,
    });
    write_report(&spec.report_path, &report)?;
    Ok(report)
}

fn artifact_fingerprints(check: &Value) -> Value {
    let mut output = serde_json::Map::new();
    let Some(entries) = check.get("viewports").and_then(Value::as_array) else {
        return Value::Object(output);
    };
    for entry in entries {
        let Some(path) = entry.get("shot").and_then(Value::as_str) else {
            continue;
        };
        let Ok(bytes) = fs::read(path) else { continue };
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for byte in bytes {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        output.insert(path.to_string(), json!(format!("fnv1a64:{hash:016x}")));
    }
    Value::Object(output)
}

fn dispatch_check(spec: &VerifySpec) -> Result<Value, String> {
    let shot_path = spec.artifacts_dir.join("screenshot.png");
    let mut request = Request::Check {
        url: Some(spec.url.clone()),
        render: None,
        base: None,
        eval: Some(spec.assertion_js.clone()),
        shot: true,
        shot_path: Some(shot_path.to_string_lossy().into_owned()),
        full: spec.full,
        baseline: None,
        tolerance: None,
        heatmap: None,
        until: LoadStage::Settled,
        keep: false,
        timeout_ms: spec.timeout_ms,
        viewports: spec.viewports.clone(),
        baseline_dir: None,
    };
    crate::normalize_request_paths(&mut request);
    match transact(&request)? {
        Response::Ok {
            value: Some(value), ..
        } => Ok(value),
        Response::Ok { .. } => Err("check returned no value".into()),
        Response::Err { message } => Err(format!("check failed: {message}")),
    }
}

fn inspect_check(check: &Value, spec: &VerifySpec, findings: &mut Vec<String>) {
    let Some(entries) = check.get("viewports").and_then(Value::as_array) else {
        findings.push("check returned no viewport evidence".into());
        return;
    };
    if entries.len() != spec.viewports.len() {
        findings.push(format!(
            "check returned {} viewport entries; expected {}",
            entries.len(),
            spec.viewports.len()
        ));
    }
    for (index, entry) in entries.iter().enumerate() {
        let size = entry
            .get("size")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        if let Some(expected) = spec.viewports.get(index) {
            let expected = expected.label();
            if size != expected {
                findings.push(format!(
                    "viewport entry {index} has size {size}; expected {expected}"
                ));
            }
        }
        match entry.get("shot") {
            Some(Value::String(path)) if Path::new(path).exists() => {}
            Some(value) => findings.push(format!("viewport {size} screenshot failed: {value}")),
            None => findings.push(format!("viewport {size} returned no screenshot")),
        }
        inspect_assertion(entry.get("eval"), &format!("viewport {size}"), findings);
    }
    if spec.fail_on_console {
        if let Some(entries) = check.get("console").and_then(Value::as_array) {
            let failures = entries
                .iter()
                .filter(|entry| {
                    matches!(
                        entry.get("kind").and_then(Value::as_str),
                        Some("exception" | "network")
                    ) || entry.get("level").and_then(Value::as_str) == Some("error")
                })
                .count();
            if failures > 0 {
                findings.push(format!("captured {failures} console/network error(s)"));
            }
        }
    }
}

fn inspect_assertion(value: Option<&Value>, label: &str, findings: &mut Vec<String>) {
    let Some(value) = value else {
        findings.push(format!("{label} returned no assertion result"));
        return;
    };
    if let Some(error) = value.get("error") {
        findings.push(format!("{label} assertion errored: {error}"));
        return;
    }
    if value.get("ok").and_then(Value::as_bool) == Some(false) {
        findings.push(format!("{label} assertion returned ok=false"));
    }
    if value.get("horizontal_overflow").and_then(Value::as_bool) == Some(true) {
        let pixels = value
            .get("overflow_px")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        findings.push(format!("{label} has {pixels}px horizontal overflow"));
    }
}

fn run_command(command: &CommandSpec, cwd: &Path, log_path: &Path) -> Result<Value, String> {
    let started = Instant::now();
    let log = File::create(log_path)
        .map_err(|e| format!("create command log {}: {e}", log_path.display()))?;
    let stderr = log
        .try_clone()
        .map_err(|e| format!("clone command log: {e}"))?;
    let mut child = Command::new(&command.argv[0])
        .args(&command.argv[1..])
        .current_dir(cwd)
        .process_group(0)
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(stderr))
        .spawn()
        .map_err(|e| format!("start {:?}: {e}", command.argv))?;
    let deadline = started + Duration::from_millis(command.timeout_ms);
    let status = loop {
        if cancellation_requested() {
            terminate_process_group(&mut child);
            break None;
        }
        if let Some(status) = child.try_wait().map_err(|e| format!("wait command: {e}"))? {
            terminate_process_group(&mut child);
            break Some(status);
        }
        if Instant::now() >= deadline {
            terminate_process_group(&mut child);
            break None;
        }
        std::thread::sleep(Duration::from_millis(POLL_MS));
    };
    Ok(json!({
        "argv": command.argv,
        "ok": status.as_ref().is_some_and(std::process::ExitStatus::success),
        "exit_code": status.and_then(|s| s.code()),
        "timed_out": status.is_none(),
        "duration_ms": started.elapsed().as_millis() as u64,
        "log": log_path,
    }))
}

struct ServerGuard {
    child: Option<Child>,
    reused: bool,
    ready_ms: u64,
    log_path: Option<PathBuf>,
    argv: Option<Vec<String>>,
}

impl ServerGuard {
    fn prepare(spec: &VerifySpec) -> Result<Self, String> {
        let started = Instant::now();
        if endpoint_ready(&spec.url, Duration::from_millis(250)) {
            return Ok(Self {
                child: None,
                reused: true,
                ready_ms: started.elapsed().as_millis() as u64,
                log_path: None,
                argv: None,
            });
        }
        let Some(command) = &spec.server else {
            return Err(format!(
                "{} is not reachable and the verify spec has no `server` command",
                spec.url
            ));
        };
        let log_path = spec.artifacts_dir.join("server.log");
        let log = File::create(&log_path)
            .map_err(|e| format!("create server log {}: {e}", log_path.display()))?;
        let stderr = log
            .try_clone()
            .map_err(|e| format!("clone server log: {e}"))?;
        let mut child = Command::new(&command.argv[0])
            .args(&command.argv[1..])
            .current_dir(&spec.cwd)
            .process_group(0)
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(stderr))
            .spawn()
            .map_err(|e| format!("start server {:?}: {e}", command.argv))?;
        let deadline = started + Duration::from_millis(spec.ready_timeout_ms);
        loop {
            if cancellation_requested() {
                terminate_process_group(&mut child);
                return Err("verification cancelled while waiting for the server".into());
            }
            if endpoint_ready(&spec.url, Duration::from_millis(250)) {
                return Ok(Self {
                    child: Some(child),
                    reused: false,
                    ready_ms: started.elapsed().as_millis() as u64,
                    log_path: Some(log_path),
                    argv: Some(command.argv.clone()),
                });
            }
            if let Some(status) = child.try_wait().map_err(|e| format!("wait server: {e}"))? {
                terminate_process_group(&mut child);
                return Err(format!(
                    "server exited before readiness with {status}; see {}",
                    log_path.display()
                ));
            }
            if Instant::now() >= deadline {
                terminate_process_group(&mut child);
                return Err(format!(
                    "server did not make {} reachable within {} ms; see {}",
                    spec.url,
                    spec.ready_timeout_ms,
                    log_path.display()
                ));
            }
            std::thread::sleep(Duration::from_millis(POLL_MS));
        }
    }

    fn report(&self) -> Value {
        json!({
            "reused": self.reused,
            "started": self.child.is_some(),
            "ready_ms": self.ready_ms,
            "argv": self.argv,
            "log": self.log_path,
        })
    }

    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            terminate_process_group(&mut child);
        }
    }
}

impl Drop for ServerGuard {
    fn drop(&mut self) {
        self.stop();
    }
}

fn terminate_process_group(child: &mut Child) {
    // Every owned command is its own process-group leader, so package runners
    // (`npm`, `bun`, etc.) cannot leave their server or script children behind.
    let group = child.id();
    signal_process_group(group, SIGTERM);
    for _ in 0..20 {
        // Reap the group leader before probing the process group. An exited but
        // unreaped leader is still visible to kill(2), which otherwise burns the
        // entire 500 ms grace period even when there are no surviving children.
        let leader_exited = child.try_wait().ok().flatten().is_some();
        let alive = signal_process_group(group, 0);
        if !alive {
            if !leader_exited {
                let _ = child.wait();
            }
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    signal_process_group(group, SIGKILL);
    let _ = child.kill();
    let _ = child.wait();
}

const SIGKILL: i32 = 9;
const SIGINT: i32 = 2;
const SIGTERM: i32 = 15;
const SIG_ERR: usize = usize::MAX;

unsafe extern "C" {
    fn kill(pid: i32, signal: i32) -> i32;
    fn signal(signal: i32, handler: usize) -> usize;
    fn siginterrupt(signal: i32, flag: i32) -> i32;
}

fn signal_process_group(group: u32, signal: i32) -> bool {
    let Ok(group) = i32::try_from(group) else {
        return false;
    };
    // Negative PIDs address the whole POSIX process group.
    unsafe { kill(-group, signal) == 0 }
}

unsafe extern "C" fn mark_cancelled(_signal: i32) {
    CANCELLATION_REQUESTED.store(true, Ordering::SeqCst);
}

struct CancellationSignals {
    previous_int: usize,
    previous_term: usize,
}

impl CancellationSignals {
    fn install() -> Result<Self, String> {
        CANCELLATION_REQUESTED.store(false, Ordering::SeqCst);
        let handler = mark_cancelled as *const () as usize;
        let previous_int = unsafe { signal(SIGINT, handler) };
        if previous_int == SIG_ERR {
            return Err("install SIGINT cleanup handler failed".into());
        }
        let previous_term = unsafe { signal(SIGTERM, handler) };
        if previous_term == SIG_ERR {
            unsafe {
                signal(SIGINT, previous_int);
            }
            return Err("install SIGTERM cleanup handler failed".into());
        }
        // Interrupt a blocking daemon read so stack unwinding promptly drops
        // ServerGuard and terminates the owned process group.
        unsafe {
            siginterrupt(SIGINT, 1);
            siginterrupt(SIGTERM, 1);
        }
        Ok(Self {
            previous_int,
            previous_term,
        })
    }
}

impl Drop for CancellationSignals {
    fn drop(&mut self) {
        unsafe {
            signal(SIGINT, self.previous_int);
            signal(SIGTERM, self.previous_term);
        }
    }
}

pub(crate) fn cancellation_requested() -> bool {
    CANCELLATION_REQUESTED.load(Ordering::SeqCst)
}

fn endpoint_ready(url: &str, timeout: Duration) -> bool {
    let Some((host, port)) = url_authority(url) else {
        return false;
    };
    let Ok(mut addresses) = (host.as_str(), port).to_socket_addrs() else {
        return false;
    };
    addresses.any(|address| TcpStream::connect_timeout(&address, timeout).is_ok())
}

fn url_authority(url: &str) -> Option<(String, u16)> {
    let (rest, default_port) = if let Some(rest) = url.strip_prefix("http://") {
        (rest, 80)
    } else {
        let rest = url.strip_prefix("https://")?;
        (rest, 443)
    };
    let authority = rest.split('/').next()?;
    if let Some(host) = authority.strip_prefix('[') {
        let end = host.find(']')?;
        let name = &host[..end];
        let suffix = &host[end + 1..];
        let port = suffix
            .strip_prefix(':')
            .and_then(|v| v.parse().ok())
            .unwrap_or(default_port);
        return Some((name.to_string(), port));
    }
    match authority.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => Some((host.to_string(), port.parse().ok()?)),
        _ => Some((authority.to_string(), default_port)),
    }
}

fn source_fingerprint(spec: &VerifySpec) -> Result<String, String> {
    let mut files = spec.source_files.clone();
    files.sort();
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for relative in files {
        let path = absolutize(&spec.cwd, &relative);
        let bytes = match &spec.source_root {
            Some(root) => read_inline_source(root, &path)?,
            None => {
                fs::read(&path).map_err(|e| format!("read source file {}: {e}", path.display()))?
            }
        };
        for byte in relative.to_string_lossy().bytes().chain([0]).chain(bytes) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    Ok(format!("fnv1a64:{hash:016x}"))
}

fn read_inline_source(root: &Path, path: &Path) -> Result<Vec<u8>, String> {
    const O_NONBLOCK: i32 = 0o4000;
    const O_NOFOLLOW: i32 = 0o400000;

    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(O_NONBLOCK | O_NOFOLLOW)
        .open(path)
        .map_err(|e| format!("open inline source file {}: {e}", path.display()))?;
    let metadata = file
        .metadata()
        .map_err(|e| format!("inspect inline source file {}: {e}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!(
            "inline source file {} is not a regular file",
            path.display()
        ));
    }
    let fd_path = PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd()));
    let opened_path = fs::canonicalize(&fd_path)
        .map_err(|e| format!("resolve opened inline source {}: {e}", path.display()))?;
    if !opened_path.starts_with(root) {
        return Err(format!(
            "opened inline source {} escaped the MCP working directory",
            path.display()
        ));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|e| format!("read inline source file {}: {e}", path.display()))?;
    Ok(bytes)
}

fn spec_fingerprint(spec: &VerifySpec) -> String {
    let command = |value: &Option<CommandSpec>| match value {
        Some(value) => json!({ "argv": value.argv, "timeout_ms": value.timeout_ms }),
        None => Value::Null,
    };
    let canonical = json!({
        "version": SCHEMA_VERSION,
        "name": spec.name,
        "cwd": spec.cwd,
        "url": spec.url,
        "tier": spec.tier.as_str(),
        "preflight": command(&spec.preflight),
        "server": command(&spec.server),
        "ready_timeout_ms": spec.ready_timeout_ms,
        "viewports": spec.viewports,
        "assertion_js": spec.assertion_js,
        "full": spec.full,
        "fail_on_console": spec.fail_on_console,
        "source_files": spec.source_files,
        "artifacts_dir": spec.artifacts_dir,
        "report_path": spec.report_path,
        "timeout_ms": spec.timeout_ms,
    });
    let bytes = serde_json::to_vec(&canonical).expect("serialize parsed verify spec");
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("fnv1a64:{hash:016x}")
}

fn write_report(path: &Path, report: &Value) -> Result<(), String> {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("report.json");
    let temp = path.with_file_name(format!(".{name}.tmp-{}", std::process::id()));
    let result = (|| {
        let mut file =
            File::create(&temp).map_err(|e| format!("create report {}: {e}", temp.display()))?;
        serde_json::to_writer_pretty(&mut file, report)
            .map_err(|e| format!("write report {}: {e}", temp.display()))?;
        file.write_all(b"\n")
            .map_err(|e| format!("finish report {}: {e}", temp.display()))?;
        file.sync_all()
            .map_err(|e| format!("sync report {}: {e}", temp.display()))?;
        fs::rename(&temp, path).map_err(|e| format!("publish report {}: {e}", path.display()))
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

struct Lease {
    path: PathBuf,
}

impl Lease {
    fn acquire(spec: &VerifySpec) -> Result<Self, String> {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("hwatu")
            .join("verify");
        fs::create_dir_all(&runtime)
            .map_err(|e| format!("create verification lease dir {}: {e}", runtime.display()))?;
        let key = fnv(&format!(
            "{}\0{}\0{}",
            spec.cwd.display(),
            spec.name,
            spec.url
        ));
        let path = runtime.join(format!("{key:016x}.lock"));
        for _ in 0..2 {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    writeln!(file, "{}", std::process::id()).map_err(|e| e.to_string())?;
                    return Ok(Self { path });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let pid = fs::read_to_string(&path)
                        .ok()
                        .and_then(|v| v.trim().parse::<u32>().ok());
                    let alive = pid.is_some_and(|pid| Path::new(&format!("/proc/{pid}")).exists());
                    if alive {
                        return Err(format!(
                            "verification job {:?} is already running (lease {})",
                            spec.name,
                            path.display()
                        ));
                    }
                    let _ = fs::remove_file(&path);
                }
                Err(error) => return Err(format!("create verification lease: {error}")),
            }
        }
        Err("could not acquire verification lease".into())
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn fnv(value: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in value.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn transact(request: &Request) -> Result<Response, String> {
    let mut stream =
        crate::connect_or_spawn().map_err(|e| format!("cannot reach hwatu daemon: {e}"))?;
    let mut payload = serde_json::to_vec(request).map_err(|e| e.to_string())?;
    payload.push(b'\n');
    stream
        .write_all(&payload)
        .map_err(|e| format!("write failed: {e}"))?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|e| format!("read failed: {e}"))?;
    serde_json::from_str(line.trim()).map_err(|e| format!("bad daemon response: {e} ({line:?})"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_spec() -> Value {
        json!({
            "version": 1,
            "name": "harness-chore",
            "url": "http://127.0.0.1:4321/about",
            "source_files": ["src/page.html"]
        })
    }

    #[test]
    fn minimal_spec_gets_fast_responsive_defaults() {
        let spec = VerifySpec::parse(base_spec(), Path::new("/repo")).unwrap();
        assert_eq!(spec.tier, Tier::Micro);
        assert_eq!(spec.viewports.len(), 3);
        assert!(!spec.full);
        assert_eq!(spec.cwd, Path::new("/repo"));
        assert_eq!(
            spec.report_path,
            Path::new("/repo/.hwatu/verify/harness-chore/report.json")
        );
    }

    #[test]
    fn interactive_jobs_require_an_explicit_assertion() {
        let mut value = base_spec();
        value["tier"] = json!("interactive");
        assert!(VerifySpec::parse(value, Path::new("/repo"))
            .unwrap_err()
            .contains("require `assertion_js`"));
    }

    #[test]
    fn command_specs_are_argv_not_shell_strings() {
        let mut value = base_spec();
        value["preflight"] = json!({ "argv": ["npm", "run", "quality:ui"] });
        let spec = VerifySpec::parse(value, Path::new("/repo")).unwrap();
        assert_eq!(spec.preflight.unwrap().argv[0], "npm");
    }

    #[test]
    fn invalid_job_names_cannot_select_paths() {
        let mut value = base_spec();
        value["name"] = json!("../escape");
        assert!(VerifySpec::parse(value, Path::new("/repo")).is_err());
    }

    #[test]
    fn unknown_fields_cannot_silently_weaken_a_job() {
        let mut value = base_spec();
        value["ready_timout_ms"] = json!(1);
        assert!(VerifySpec::parse(value, Path::new("/repo"))
            .unwrap_err()
            .contains("unknown verify spec field"));
    }

    #[test]
    fn authority_parser_handles_ports_and_ipv6() {
        assert_eq!(
            url_authority("http://127.0.0.1:4321/x"),
            Some(("127.0.0.1".into(), 4321))
        );
        assert_eq!(
            url_authority("https://example.com/x"),
            Some(("example.com".into(), 443))
        );
        assert_eq!(
            url_authority("http://[::1]:9000/x"),
            Some(("::1".into(), 9000))
        );
    }

    #[test]
    fn assertion_failures_are_structured_findings() {
        let mut findings = Vec::new();
        inspect_assertion(
            Some(&json!({ "ok": false, "horizontal_overflow": true, "overflow_px": 9 })),
            "viewport 390x844",
            &mut findings,
        );
        assert_eq!(findings.len(), 2);
    }

    #[test]
    fn viewport_evidence_must_match_the_requested_size() {
        let mut value = base_spec();
        value["viewports"] = json!(["390x844"]);
        let spec = VerifySpec::parse(value, Path::new("/repo")).unwrap();
        let mut findings = Vec::new();
        inspect_check(
            &json!({ "viewports": [{ "size": "1440x1000" }] }),
            &spec,
            &mut findings,
        );
        assert!(findings
            .iter()
            .any(|finding| finding.contains("expected 390x844")));
    }

    #[test]
    fn inline_mcp_specs_cannot_smuggle_process_commands() {
        let mut value = base_spec();
        value["preflight"] = json!({ "argv": ["sh", "-c", "echo unsafe"] });
        let error = validate_inline_spec(&value, Path::new("/repo")).unwrap_err();
        assert!(error.contains("cannot set `preflight`"));
    }

    #[test]
    fn inline_mcp_specs_cannot_select_filesystem_outputs_or_escape_sources() {
        let mut output = base_spec();
        output["report_path"] = json!("/tmp/overwrite-me");
        assert!(validate_inline_spec(&output, Path::new("/repo")).is_err());

        let mut source = base_spec();
        source["source_files"] = json!(["../secret"]);
        assert!(validate_inline_spec(&source, Path::new("/repo")).is_err());
    }

    #[test]
    fn inline_mcp_sources_cannot_escape_through_symlinks() {
        use std::os::unix::fs::symlink;

        let suffix = format!("{}-{}", std::process::id(), fnv(module_path!()));
        let root = std::env::temp_dir().join(format!("hwatu-inline-root-{suffix}"));
        let outside = std::env::temp_dir().join(format!("hwatu-inline-outside-{suffix}"));
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&outside);
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("source.txt"), "secret").unwrap();
        symlink(outside.join("source.txt"), root.join("source.txt")).unwrap();

        let mut value = base_spec();
        value["source_files"] = json!(["source.txt"]);
        let error = validate_inline_spec(&value, &root).unwrap_err();
        assert!(error.contains("workspace symlinks"));

        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(outside);
    }

    #[test]
    fn inline_source_swap_is_rejected_before_reading() {
        use std::os::unix::fs::symlink;

        let suffix = format!("{}-{}", std::process::id(), fnv("swap"));
        let root = std::env::temp_dir().join(format!("hwatu-inline-swap-root-{suffix}"));
        let outside = std::env::temp_dir().join(format!("hwatu-inline-swap-outside-{suffix}"));
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&outside);
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(root.join("source.txt"), "safe").unwrap();
        fs::write(outside.join("source.txt"), "secret").unwrap();

        let mut value = base_spec();
        value["source_files"] = json!(["source.txt"]);
        let (canonical_root, sources) = validate_inline_spec(&value, &root).unwrap();
        fs::remove_file(root.join("source.txt")).unwrap();
        symlink(outside.join("source.txt"), root.join("source.txt")).unwrap();
        assert!(read_inline_source(&canonical_root, &sources[0]).is_err());

        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(outside);
    }

    #[test]
    fn inline_sources_must_be_regular_files() {
        use std::os::unix::net::UnixListener;

        let root = std::env::temp_dir().join(format!(
            "hwatu-inline-socket-{}-{}",
            std::process::id(),
            fnv("socket")
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let socket = root.join("source.sock");
        let _listener = UnixListener::bind(&socket).unwrap();
        assert!(read_inline_source(&root, &socket).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn failed_reports_are_mcp_tool_errors_without_losing_evidence() {
        let report = json!({ "passed": false, "findings": ["broken"] });
        let error = serialize_mcp_report(&report).unwrap_err();
        assert_eq!(serde_json::from_str::<Value>(&error).unwrap(), report);
    }

    #[test]
    fn screenshot_evidence_gets_a_stable_fingerprint() {
        let path = std::env::temp_dir().join(format!(
            "hwatu-verify-artifact-{}-{}.png",
            std::process::id(),
            fnv(module_path!())
        ));
        fs::write(&path, b"pixels").unwrap();
        let value = artifact_fingerprints(&json!({
            "viewports": [{ "shot": path }]
        }));
        assert_eq!(value.as_object().unwrap().len(), 1);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn spec_fingerprint_changes_with_verification_semantics() {
        let first = VerifySpec::parse(base_spec(), Path::new("/repo")).unwrap();
        let mut changed = base_spec();
        changed["viewports"] = json!(["390x844"]);
        let changed = VerifySpec::parse(changed, Path::new("/repo")).unwrap();
        assert_ne!(spec_fingerprint(&first), spec_fingerprint(&changed));
    }

    #[test]
    fn clean_process_group_shutdown_does_not_wait_the_full_grace_period() {
        let mut child = Command::new("sleep")
            .arg("30")
            .process_group(0)
            .spawn()
            .unwrap();
        let started = Instant::now();

        terminate_process_group(&mut child);

        assert!(
            started.elapsed() < Duration::from_millis(400),
            "clean shutdown took {:?}",
            started.elapsed()
        );
        assert!(!signal_process_group(child.id(), 0));
    }
}
