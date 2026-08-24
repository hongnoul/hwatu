// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Justin Hong
//! Translate client-local artifacts to and from inline TCP payloads.

use hwatu_ipc::{BatchResult, Request, Response};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};

static NEXT_ARTIFACT: AtomicU64 = AtomicU64::new(1);

// Open(2) flag values are per-kernel ABI, not POSIX: Linux uses
// 0o4000/0o400000, while the BSD family (macOS included) uses
// 0x4/0x100. Hardcoding the Linux values everywhere silently turned
// O_NOFOLLOW into O_NOCTTY on macOS, so the symlink-refusal path
// never engaged there (caught by
// `materialization_refuses_symlink_destinations` failing on Darwin).
#[cfg(target_os = "linux")]
const O_NONBLOCK: i32 = 0o4000;
#[cfg(target_os = "linux")]
const O_NOFOLLOW: i32 = 0o400000;
#[cfg(all(unix, not(target_os = "linux")))]
const O_NONBLOCK: i32 = 0x0004;
#[cfg(all(unix, not(target_os = "linux")))]
const O_NOFOLLOW: i32 = 0x0100;

#[derive(Default)]
pub(crate) struct Artifacts {
    response_data: Option<Destination>,
    check_shots: Option<ShotDestinations>,
    heatmap: Option<Destination>,
    batch: Option<Vec<Artifacts>>,
}

enum ShotDestinations {
    Single(Destination),
    Sweep { base: Option<PathBuf> },
}

struct Destination {
    path: PathBuf,
    exclusive: bool,
}

pub(crate) fn prepare(request: &mut Request) -> Result<Artifacts, String> {
    let mut artifacts = Artifacts::default();
    match request {
        Request::Screenshot { path, data, .. } => {
            artifacts.response_data = Some(destination(path.take(), "screenshot", "png")?);
            *data = true;
        }
        Request::Check {
            shot,
            shot_path,
            shot_data,
            baseline,
            baseline_data,
            heatmap,
            heatmap_data,
            viewports,
            baseline_dir,
            ..
        } => {
            if baseline_dir.is_some() {
                return Err(
                    "TCP checks do not support baseline_dir; run separate checks with inline baselines"
                        .to_string(),
                );
            }
            if baseline.is_some() && baseline_data.is_some() {
                return Err("baseline path and inline baseline data are mutually exclusive".into());
            }
            let wants_shot = *shot || shot_path.is_some() || *shot_data;
            let wants_heatmap = heatmap.is_some() || *heatmap_data;
            if wants_shot && wants_heatmap {
                return Err(
                    "one TCP check cannot return both screenshot and heatmap data; request them separately"
                        .to_string(),
                );
            }
            if wants_shot && viewports.len() > 1 {
                return Err(
                    "TCP screenshot checks support one viewport per request; send each viewport separately"
                        .to_string(),
                );
            }
            if let Some(path) = baseline.take() {
                *baseline_data = Some(read_inline_file(&path, "baseline")?);
            }
            if wants_shot {
                *shot_data = true;
                if viewports.is_empty() {
                    artifacts.check_shots = Some(ShotDestinations::Single(destination(
                        shot_path.take(),
                        "check-shot",
                        "png",
                    )?));
                } else {
                    artifacts.check_shots = Some(ShotDestinations::Sweep {
                        base: shot_path.take().map(PathBuf::from),
                    });
                }
            }
            if wants_heatmap {
                artifacts.heatmap = Some(destination(heatmap.take(), "heatmap", "png")?);
                *heatmap_data = true;
            }
        }
        Request::Upload { path, data, .. } => {
            if data.is_none() {
                *data = Some(read_inline_file(path, "upload")?);
            }
        }
        Request::Diff {
            baseline,
            baseline_data,
            heatmap,
            heatmap_data,
            ..
        } => {
            if baseline.is_some() && baseline_data.is_some() {
                return Err("baseline path and inline baseline data are mutually exclusive".into());
            }
            if let Some(path) = baseline.take() {
                *baseline_data = Some(read_inline_file(&path, "baseline")?);
            }
            if heatmap.is_some() || *heatmap_data {
                artifacts.heatmap = Some(destination(heatmap.take(), "heatmap", "png")?);
                *heatmap_data = true;
            }
        }
        Request::Batch { actions } => {
            let mut plans = Vec::with_capacity(actions.len());
            let mut outputs = 0usize;
            for action in actions {
                let plan = prepare(action)?;
                outputs += plan.output_count();
                plans.push(plan);
            }
            if outputs > 1 {
                return Err(
                    "a TCP batch can return at most one screenshot or heatmap artifact".to_string(),
                );
            }
            artifacts.batch = Some(plans);
        }
        _ => {}
    }
    Ok(artifacts)
}

impl Artifacts {
    pub(crate) fn materialize(self, response: &mut Response) -> Result<(), String> {
        let Response::Ok {
            data, value, path, ..
        } = response
        else {
            return Ok(());
        };

        if let Some(plans) = self.batch {
            let value = value
                .as_mut()
                .ok_or_else(|| "daemon batch response omitted its value".to_string())?;
            let encoded = value
                .get_mut("batch")
                .ok_or_else(|| "daemon response omitted batch results".to_string())?;
            let mut batch: BatchResult = serde_json::from_value(encoded.take())
                .map_err(|error| format!("daemon returned invalid batch results: {error}"))?;
            for (plan, step) in plans.into_iter().zip(&mut batch.steps) {
                if let Some(step_response) = step.response.as_mut() {
                    plan.materialize(step_response)?;
                }
            }
            *encoded = serde_json::to_value(batch)
                .map_err(|error| format!("cannot encode materialized batch results: {error}"))?;
            return Ok(());
        }

        if let Some(destination) = self.response_data {
            let encoded = data
                .take()
                .ok_or_else(|| "daemon response omitted inline screenshot data".to_string())?;
            let written = destination.write(&encoded)?;
            *path = Some(written);
        }

        let Some(value) = value.as_mut() else {
            return Ok(());
        };
        if let Some(shots) = self.check_shots {
            match shots {
                ShotDestinations::Single(destination) => {
                    let Some(encoded) = take_string(value, "shot_data")? else {
                        if nested_error(value, "shot") {
                            return Ok(());
                        }
                        return Err(
                            "daemon check response omitted inline screenshot data".to_string()
                        );
                    };
                    let written = destination.write(&encoded)?;
                    value["shot"] = serde_json::Value::String(written);
                }
                ShotDestinations::Sweep { base } => {
                    let entries = value
                        .get_mut("viewports")
                        .and_then(serde_json::Value::as_array_mut)
                        .ok_or_else(|| "daemon response omitted viewport results".to_string())?;
                    for entry in entries {
                        let Some(encoded) = take_string(entry, "shot_data")? else {
                            continue;
                        };
                        let label = entry
                            .get("size")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("viewport");
                        let destination = match base.as_deref() {
                            Some(base) => Destination {
                                path: per_size_path(base, label),
                                exclusive: false,
                            },
                            None => destination(None, &format!("check-{label}"), "png")?,
                        };
                        entry["shot"] = serde_json::Value::String(destination.write(&encoded)?);
                    }
                }
            }
        }
        if let Some(destination) = self.heatmap {
            let target = if value.get("diff").is_some() {
                value.get_mut("diff").expect("presence checked")
            } else {
                value
            };
            if target.get("error").is_some() {
                return Ok(());
            }
            let encoded = take_string(target, "heatmap_data")?
                .ok_or_else(|| "daemon response omitted inline heatmap data".to_string())?;
            target["heatmap"] = serde_json::Value::String(destination.write(&encoded)?);
        }
        Ok(())
    }

    fn output_count(&self) -> usize {
        usize::from(self.response_data.is_some())
            + usize::from(self.check_shots.is_some())
            + usize::from(self.heatmap.is_some())
            + self
                .batch
                .as_ref()
                .map(|plans| plans.iter().map(Artifacts::output_count).sum())
                .unwrap_or(0)
    }
}

fn nested_error(value: &serde_json::Value, key: &str) -> bool {
    value
        .get(key)
        .and_then(serde_json::Value::as_object)
        .is_some_and(|object| object.contains_key("error"))
}

fn take_string(value: &mut serde_json::Value, key: &str) -> Result<Option<String>, String> {
    let Some(object) = value.as_object_mut() else {
        return Err("daemon artifact response was not an object".to_string());
    };
    match object.remove(key) {
        Some(serde_json::Value::String(value)) => Ok(Some(value)),
        Some(_) => Err(format!("daemon artifact field {key:?} was not a string")),
        None => Ok(None),
    }
}

fn read_inline_file(path: &str, label: &str) -> Result<String, String> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(O_NONBLOCK | O_NOFOLLOW);
    let file = options
        .open(path)
        .map_err(|error| format!("cannot open {label} file {path}: {error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("cannot inspect {label} file {path}: {error}"))?;
    if !metadata.is_file() {
        return Err(format!("{label} path {path} is not a regular file"));
    }
    if metadata.len() > hwatu_ipc::INLINE_MAX_BYTES as u64 {
        return Err(format!(
            "{label} file {path} is {} bytes; inline limit is {} bytes",
            metadata.len(),
            hwatu_ipc::INLINE_MAX_BYTES
        ));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(hwatu_ipc::INLINE_MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read {label} file {path}: {error}"))?;
    if bytes.len() > hwatu_ipc::INLINE_MAX_BYTES {
        return Err(format!(
            "{label} file {path} grew beyond the inline limit while being read"
        ));
    }
    Ok(hwatu_ipc::base64::encode(&bytes))
}

fn destination(path: Option<String>, stem: &str, extension: &str) -> Result<Destination, String> {
    match path {
        Some(path) => Ok(Destination {
            path: PathBuf::from(path),
            exclusive: false,
        }),
        None => Ok(Destination {
            path: generated_path(stem, extension)?,
            exclusive: true,
        }),
    }
}

impl Destination {
    fn write(self, encoded: &str) -> Result<String, String> {
        let bytes = hwatu_ipc::base64::decode(encoded)
            .map_err(|error| format!("daemon returned invalid base64 artifact: {error}"))?;
        if bytes.len() > hwatu_ipc::INLINE_MAX_BYTES {
            return Err(format!(
                "daemon artifact is {} bytes; inline limit is {} bytes",
                bytes.len(),
                hwatu_ipc::INLINE_MAX_BYTES
            ));
        }
        let mut options = std::fs::OpenOptions::new();
        options.write(true);
        if self.exclusive {
            options.create_new(true);
        } else {
            options.create(true).truncate(true);
        }
        #[cfg(unix)]
        {
            options.mode(0o600);
            options.custom_flags(O_NOFOLLOW);
        }
        let mut file = options.open(&self.path).map_err(|error| {
            format!(
                "cannot create local artifact {}: {error}",
                self.path.display()
            )
        })?;
        file.write_all(&bytes).map_err(|error| {
            format!(
                "cannot write local artifact {}: {error}",
                self.path.display()
            )
        })?;
        Ok(self.path.to_string_lossy().into_owned())
    }
}

fn generated_path(stem: &str, extension: &str) -> Result<PathBuf, String> {
    let root = client_artifact_root()?;
    for _ in 0..32 {
        let sequence = NEXT_ARTIFACT.fetch_add(1, Ordering::Relaxed);
        let path = root.join(format!(
            "{stem}-{}-{sequence}.{extension}",
            std::process::id()
        ));
        if !path.exists() {
            return Ok(path);
        }
    }
    Err("could not allocate a unique local artifact path".to_string())
}

fn client_artifact_root() -> Result<PathBuf, String> {
    let root = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("hwatu-client-{}", effective_uid()));
    match std::fs::symlink_metadata(&root) {
        Ok(metadata) => {
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(format!(
                    "artifact path {} is not a real directory",
                    root.display()
                ));
            }
            #[cfg(unix)]
            {
                if metadata.uid() != effective_uid() {
                    return Err(format!(
                        "artifact directory {} is owned by another user",
                        root.display()
                    ));
                }
                if metadata.permissions().mode() & 0o077 != 0 {
                    return Err(format!(
                        "artifact directory {} has unsafe permissions",
                        root.display()
                    ));
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            builder.mode(0o700);
            builder.create(&root).map_err(|error| {
                format!(
                    "cannot create artifact directory {}: {error}",
                    root.display()
                )
            })?;
        }
        Err(error) => {
            return Err(format!(
                "cannot inspect artifact directory {}: {error}",
                root.display()
            ));
        }
    }
    Ok(root)
}

fn per_size_path(path: &Path, label: &str) -> PathBuf {
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("shot");
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("png");
    let name = format!("{stem}-{label}.{extension}");
    if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        dir.join(name)
    } else {
        PathBuf::from(name)
    }
}

#[cfg(unix)]
fn effective_uid() -> u32 {
    unsafe { geteuid() }
}

#[cfg(unix)]
extern "C" {
    fn geteuid() -> u32;
}

#[cfg(not(unix))]
fn effective_uid() -> u32 {
    std::process::id()
}

#[cfg(test)]
mod tests {
    use super::{prepare, Artifacts, Destination, ShotDestinations};
    use hwatu_ipc::{BatchResult, BatchStepResult, BatchStepStatus, Request, Response};

    fn output_path(stem: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "hwatu-{stem}-test-{}-{}",
            std::process::id(),
            super::NEXT_ARTIFACT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ))
    }

    #[test]
    fn screenshot_roundtrip_materializes_bytes_and_removes_base64() {
        let output = output_path("remote-shot").with_extension("png");
        let mut request = Request::Screenshot {
            id: None,
            path: Some(output.to_string_lossy().into_owned()),
            full: false,
            data: false,
        };
        let artifacts = prepare(&mut request).unwrap();
        assert!(matches!(
            request,
            Request::Screenshot {
                path: None,
                data: true,
                ..
            }
        ));
        let mut response = Response::data(hwatu_ipc::base64::encode(b"png bytes"));
        artifacts.materialize(&mut response).unwrap();
        assert_eq!(std::fs::read(&output).unwrap(), b"png bytes");
        assert!(matches!(
            response,
            Response::Ok {
                data: None,
                path: Some(_),
                ..
            }
        ));
        std::fs::remove_file(output).unwrap();
    }

    #[test]
    fn empty_artifact_plan_leaves_errors_untouched() {
        let mut response = Response::err("nope");
        Artifacts::default().materialize(&mut response).unwrap();
        assert!(matches!(response, Response::Err { .. }));
    }

    #[test]
    fn nested_capture_errors_are_not_replaced_by_materialization_errors() {
        let output = output_path("nested-error");
        let artifacts = Artifacts {
            check_shots: Some(ShotDestinations::Single(Destination {
                path: output.clone(),
                exclusive: false,
            })),
            ..Artifacts::default()
        };
        let mut response = Response::value(serde_json::json!({
            "shot": { "error": "capture failed" }
        }));
        artifacts.materialize(&mut response).unwrap();
        let Response::Ok {
            value: Some(value), ..
        } = response
        else {
            panic!("expected structured response");
        };
        assert_eq!(value["shot"]["error"], "capture failed");
        assert!(!output.exists());
    }

    #[test]
    fn tcp_batch_materializes_its_single_artifact() {
        let output = output_path("batch-shot").with_extension("png");
        let mut request = Request::Batch {
            actions: vec![Request::Screenshot {
                id: None,
                path: Some(output.to_string_lossy().into_owned()),
                full: false,
                data: false,
            }],
        };
        let artifacts = prepare(&mut request).unwrap();
        let batch = BatchResult {
            complete: true,
            executed: 1,
            failed_at: None,
            steps: vec![BatchStepResult {
                index: 0,
                action: "screenshot".to_string(),
                status: BatchStepStatus::Ok,
                response: Some(Response::data(hwatu_ipc::base64::encode(b"batch png"))),
                error: None,
                skipped_reason: None,
            }],
        };
        let mut response = Response::value(serde_json::json!({ "batch": batch }));
        artifacts.materialize(&mut response).unwrap();
        assert_eq!(std::fs::read(&output).unwrap(), b"batch png");
        let Response::Ok {
            value: Some(value), ..
        } = response
        else {
            panic!("expected batch response");
        };
        let result: BatchResult = serde_json::from_value(value["batch"].clone()).unwrap();
        assert!(matches!(
            result.steps[0].response,
            Some(Response::Ok {
                data: None,
                path: Some(_),
                ..
            })
        ));
        std::fs::remove_file(output).unwrap();
    }

    #[test]
    fn tcp_batch_rejects_multiple_artifact_outputs() {
        let mut request = Request::Batch {
            actions: vec![
                Request::Screenshot {
                    id: Some(1),
                    path: None,
                    full: false,
                    data: false,
                },
                Request::Screenshot {
                    id: Some(2),
                    path: None,
                    full: false,
                    data: false,
                },
            ],
        };
        let error = prepare(&mut request).err().expect("batch must be rejected");
        assert!(error.contains("at most one"));
    }

    #[cfg(unix)]
    #[test]
    fn materialization_refuses_symlink_destinations() {
        let target = output_path("symlink-target");
        let link = output_path("symlink-output");
        std::fs::write(&target, b"keep").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let mut request = Request::Screenshot {
            id: None,
            path: Some(link.to_string_lossy().into_owned()),
            full: false,
            data: false,
        };
        let artifacts = prepare(&mut request).unwrap();
        let mut response = Response::data(hwatu_ipc::base64::encode(b"replace"));
        assert!(artifacts.materialize(&mut response).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"keep");
        std::fs::remove_file(link).unwrap();
        std::fs::remove_file(target).unwrap();
    }
}
