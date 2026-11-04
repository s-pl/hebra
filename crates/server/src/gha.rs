//! GitHub Actions workflows (`.github/workflows/*.yml`) as Hebra checks,
//! for repositories without `.hb/checks.toml`: teams coming from GitHub
//! keep their CI file.
//!
//! What maps: `on` (push with branch filters, pull_request as reviews),
//! jobs with `needs`, `runs-on` (as labels: `ubuntu-*` → `linux`,
//! `windows-*` → `windows`, `macos-*` → `macos`, custom self-hosted labels
//! as they are), `strategy.matrix` (plain lists), `container` and
//! `services`, `env`, `environment`, `timeout-minutes`, and `run` steps with
//! their `name`, `env`, `working-directory` and `if: always()` or
//! `failure()`. `${{ matrix.x }}` works as in Hebra; `${{ secrets.X }}`
//! becomes the variable `$X` and the check asks for that secret;
//! `${{ github.sha }}` and a few others become Hebra's variables.
//!
//! Actions (`uses:`) cannot run here: `actions/checkout` is not needed (the
//! commit is checked out), `actions/upload-artifact` becomes `artifacts`,
//! `actions/cache` becomes `cache`, and any other is skipped with a note in
//! the log.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::checks::CheckSpec;
use crate::error::{ApiError, ApiResult};

/// `on:` as Hebra triggers.
fn triggers(on: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let mut push = |branches: Option<&Value>| match branches.and_then(Value::as_array) {
        Some(list) if !list.is_empty() => {
            for b in list.iter().filter_map(Value::as_str) {
                out.push(format!("push:{}", b.replace("**", "*")));
            }
        }
        _ => out.push("push:*".to_owned()),
    };
    match on {
        Value::String(s) if s == "push" => push(None),
        Value::String(s) if s == "pull_request" || s == "pull_request_target" => {}
        Value::Array(list) => {
            for e in list.iter().filter_map(Value::as_str) {
                if e == "push" {
                    push(None);
                }
            }
        }
        Value::Object(map) => {
            if let Some(p) = map.get("push") {
                push(p.get("branches"));
            }
        }
        _ => {}
    }
    let review = match on {
        Value::String(s) => s.starts_with("pull_request"),
        Value::Array(list) => list
            .iter()
            .any(|e| e.as_str().is_some_and(|s| s.starts_with("pull_request"))),
        Value::Object(map) => map.keys().any(|k| k.starts_with("pull_request")),
        _ => false,
    };
    if review {
        out.push("review".to_owned());
    }
    out.sort();
    out.dedup();
    out
}

fn labels(runs_on: &Value) -> Vec<String> {
    let list: Vec<&str> = match runs_on {
        Value::String(s) => vec![s.as_str()],
        Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
        Value::Object(o) => o
            .get("labels")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    let mut out: Vec<String> = list
        .into_iter()
        .filter(|l| *l != "self-hosted")
        .map(|l| {
            if l.starts_with("ubuntu") || l == "linux" {
                "linux".to_owned()
            } else if l.starts_with("windows") {
                "windows".to_owned()
            } else if l.starts_with("macos") {
                "macos".to_owned()
            } else {
                l.to_owned()
            }
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// GitHub's runner names as Hebra labels (also after a matrix fills them).
#[must_use]
pub fn normalize_label(label: &str) -> String {
    if label.starts_with("ubuntu") {
        "linux".to_owned()
    } else if label.starts_with("windows") {
        "windows".to_owned()
    } else if label.starts_with("macos") {
        "macos".to_owned()
    } else {
        label.to_owned()
    }
}

/// `${{ … }}` expressions a shell can take, and the secrets they name.
fn expressions(text: &str, secrets: &mut Vec<String>) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("${{") {
        out.push_str(&rest[..start]);
        let Some(end) = rest[start..].find("}}") else {
            out.push_str(&rest[start..]);
            return out;
        };
        let inner = rest[start + 3..start + end].trim();
        let replaced = if let Some(name) = inner.strip_prefix("secrets.") {
            secrets.push(name.to_owned());
            format!("${{{name}}}")
        } else if inner.starts_with("matrix.") {
            format!("${{{{ {inner} }}}}")
        } else {
            match inner {
                "github.sha" => "${HEBRA_COMMIT}".to_owned(),
                "github.repository" => "${HEBRA_REPO}".to_owned(),
                "github.run_id" => "${HEBRA_RUN}".to_owned(),
                "github.job" => "${HEBRA_CHECK}".to_owned(),
                "runner.os" => "Linux".to_owned(),
                other => format!("${{{{ {other} }}}}"),
            }
        };
        out.push_str(&replaced);
        rest = &rest[start + end + 2..];
    }
    out.push_str(rest);
    out
}

fn string_map(v: &Value, secrets: &mut Vec<String>) -> BTreeMap<String, String> {
    v.as_object()
        .map(|m| {
            m.iter()
                .map(|(k, v)| {
                    let text = v.as_str().map_or_else(|| v.to_string(), str::to_owned);
                    (k.clone(), expressions(&text, secrets))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn job_check(
    file: &str,
    id: &str,
    job: &Value,
    on: &[String],
    workflow_env: &Value,
) -> ApiResult<Value> {
    let mut secrets = Vec::new();
    let mut env = string_map(workflow_env, &mut secrets);
    env.extend(string_map(&job["env"], &mut secrets));
    let mut steps = Vec::new();
    let mut artifacts = Vec::new();
    let mut cache = Vec::new();
    for (n, step) in job["steps"].as_array().into_iter().flatten().enumerate() {
        let name = step["name"].as_str().map(str::to_owned);
        if let Some(uses) = step["uses"].as_str() {
            let action = uses.split('@').next().unwrap_or(uses);
            match action {
                "actions/checkout" => {}
                "actions/upload-artifact" => {
                    if let Some(path) = step["with"]["path"].as_str() {
                        artifacts.extend(
                            path.lines()
                                .map(str::trim)
                                .filter(|p| !p.is_empty())
                                .map(str::to_owned),
                        );
                    }
                }
                "actions/cache" => {
                    if let (Some(key), Some(path)) =
                        (step["with"]["key"].as_str(), step["with"]["path"].as_str())
                    {
                        cache.push(json!({
                            "key": expressions(key, &mut secrets),
                            "paths": path.lines().map(str::trim).filter(|p| !p.is_empty()).collect::<Vec<_>>(),
                        }));
                    }
                }
                other => steps.push(json!({
                    "name": name.clone().unwrap_or_else(|| other.to_owned()),
                    "run": format!("echo 'Hebra does not run actions: skipped {other}'"),
                })),
            }
            continue;
        }
        let Some(run) = step["run"].as_str() else {
            continue;
        };
        let mut s = json!({
            "name": name.unwrap_or_else(|| format!("step {}", n + 1)),
            "run": expressions(run, &mut secrets),
        });
        let step_env = string_map(&step["env"], &mut secrets);
        if !step_env.is_empty() {
            s["env"] = json!(step_env);
        }
        if let Some(dir) = step["working-directory"].as_str() {
            s["workdir"] = json!(dir);
        }
        match step["if"].as_str().map(str::trim) {
            Some(c) if c.contains("always()") => s["if"] = json!("always"),
            Some(c) if c.contains("failure()") => s["if"] = json!("failure"),
            _ => {}
        }
        steps.push(s);
    }
    if steps.is_empty() {
        steps.push(json!({ "name": "nothing to run", "run": "echo 'this job has only actions'" }));
    }
    let mut check = json!({
        "name": job["name"].as_str().filter(|n| !n.contains("${{")).map_or_else(|| id.to_owned(), str::to_owned),
        "on": on,
        "steps": steps,
        "labels": labels(&job["runs-on"]),
    });
    if !env.is_empty() {
        check["env"] = json!(env);
    }
    match &job["needs"] {
        Value::String(n) => check["needs"] = json!([n]),
        Value::Array(list) => check["needs"] = json!(list),
        _ => {}
    }
    if let Some(m) = job["strategy"]["matrix"].as_object() {
        let matrix: BTreeMap<String, Vec<String>> = m
            .iter()
            .filter(|(k, _)| *k != "include" && *k != "exclude")
            .filter_map(|(k, v)| {
                Some((
                    k.clone(),
                    v.as_array()?
                        .iter()
                        .map(|x| x.as_str().map_or_else(|| x.to_string(), str::to_owned))
                        .collect(),
                ))
            })
            .collect();
        if !matrix.is_empty() {
            check["matrix"] = json!(matrix);
        }
    }
    match &job["container"] {
        Value::String(image) => check["image"] = json!(image),
        Value::Object(c) => {
            if let Some(image) = c.get("image").and_then(Value::as_str) {
                check["image"] = json!(image);
            }
        }
        _ => {}
    }
    if let Some(services) = job["services"].as_object() {
        let list: Vec<Value> = services
            .iter()
            .filter_map(|(name, s)| {
                Some(json!({ "name": name, "image": s["image"].as_str()?, "env": string_map(&s["env"], &mut Vec::new()) }))
            })
            .collect();
        check["services"] = json!(list);
    }
    match &job["environment"] {
        Value::String(e) => check["environment"] = json!(e),
        Value::Object(e) => {
            if let Some(name) = e.get("name").and_then(Value::as_str) {
                check["environment"] = json!(name);
            }
        }
        _ => {}
    }
    if let Some(minutes) = job["timeout-minutes"].as_u64() {
        check["timeout"] = json!(format!("{minutes}m"));
    }
    if !artifacts.is_empty() {
        check["artifacts"] = json!(artifacts);
    }
    if !cache.is_empty() {
        check["cache"] = json!(cache);
    }
    secrets.sort();
    secrets.dedup();
    if !secrets.is_empty() {
        check["secrets"] = json!(secrets);
    }
    let _ = file;
    Ok(check)
}

/// Every job of these workflow files as a check.
pub fn translate(files: &[(String, String)]) -> ApiResult<Vec<CheckSpec>> {
    let mut out: Vec<CheckSpec> = Vec::new();
    for (path, text) in files {
        let doc: Value =
            serde_saphyr::from_str(text).map_err(|e| ApiError::bad(format!("{path}: {e}")))?;
        let on = triggers(
            doc.get("on")
                .or_else(|| doc.get("true"))
                .unwrap_or(&Value::Null),
        );
        if on.is_empty() {
            continue;
        }
        let stem = path
            .rsplit('/')
            .next()
            .unwrap_or(path)
            .trim_end_matches(".yml")
            .trim_end_matches(".yaml");
        for (id, job) in doc["jobs"].as_object().into_iter().flatten() {
            let check = job_check(path, id, job, &on, &doc["env"])?;
            let mut spec: CheckSpec = serde_json::from_value(check)
                .map_err(|e| ApiError::bad(format!("{path}: job {id}: {e}")))?;
            if out.iter().any(|c| c.name == spec.name) {
                spec.name = format!("{stem}/{}", spec.name);
            }
            out.push(spec);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORKFLOW: &str = r#"
name: CI
on:
  push:
    branches: [main, "release/**"]
  pull_request:
env:
  CARGO_TERM_COLOR: always
jobs:
  test:
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        os: [ubuntu-latest, windows-latest]
    timeout-minutes: 30
    steps:
      - uses: actions/checkout@v4
      - uses: actions/cache@v4
        with:
          key: cargo-${{ matrix.os }}
          path: |
            target
      - name: Test
        run: cargo test --all
        env:
          TOKEN: ${{ secrets.NPM_TOKEN }}
      - uses: actions/upload-artifact@v4
        with:
          path: target/report.xml
      - name: Report
        if: always()
        run: echo done
  deploy:
    needs: test
    runs-on: [self-hosted, prod]
    environment: production
    container: node:24
    steps:
      - uses: some/deploy-action@v1
      - run: ./deploy.sh ${{ github.sha }}
"#;

    #[test]
    fn workflows_become_checks() {
        let specs =
            translate(&[(".github/workflows/ci.yml".to_owned(), WORKFLOW.to_owned())]).unwrap();
        assert_eq!(specs.len(), 2);
        let by = |n: &str| specs.iter().find(|s| s.name == n).unwrap();
        let test = by("test");
        assert_eq!(test.on, vec!["push:main", "push:release/*", "review"]);
        assert_eq!(test.matrix["os"], vec!["ubuntu-latest", "windows-latest"]);
        assert_eq!(test.secrets, vec!["NPM_TOKEN"]);
        assert_eq!(test.artifacts, vec!["target/report.xml"]);
        assert_eq!(test.cache[0].key, "cargo-${{ matrix.os }}");
        assert_eq!(test.timeout.as_deref(), Some("30m"));
        assert_eq!(test.steps.last().unwrap().when.as_deref(), Some("always"));
        assert!(test.steps[0].env["TOKEN"].contains("${NPM_TOKEN}"));
        assert_eq!(test.env["CARGO_TERM_COLOR"], "always");
        let deploy = by("deploy");
        assert_eq!(deploy.needs, vec!["test"]);
        assert_eq!(deploy.labels, vec!["prod"]);
        assert_eq!(deploy.environment.as_deref(), Some("production"));
        assert_eq!(deploy.image.as_deref(), Some("node:24"));
        assert!(deploy.steps[0].run.contains("skipped some/deploy-action"));
        assert!(deploy.steps[1].run.contains("${HEBRA_COMMIT}"));
    }

    #[test]
    fn matrix_labels_map_os_names() {
        assert_eq!(labels(&json!("ubuntu-24.04")), vec!["linux"]);
        assert_eq!(labels(&json!(["self-hosted", "gpu"])), vec!["gpu"]);
    }
}
