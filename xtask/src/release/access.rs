use crate::tool::{self, Tool};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;
use std::process::Stdio;

const API: &str = "https://api.doppler.com";
const PAGE_SIZE: usize = 100;
const MAX_PAGES: usize = 250;
const REQUEST_TIMEOUT: &str = "30";
const SYNC_LIMIT: usize = 100;
const PURPOSES: [&str; 5] = ["automation", "macos", "windows", "notarize", "manifest"];

fn environment(purpose: &str) -> String {
    if purpose == "automation" {
        "release-please".to_owned()
    } else {
        format!("release-{purpose}")
    }
}

fn sync_data(purpose: &str) -> Value {
    json!({"feature":"actions", "sync_target":"repo", "repo_name":"youarel", "environment_name":environment(purpose), "sync_unmasked_as_variables":false})
}

fn has_repository(value: &Value) -> bool {
    match value {
        Value::String(text) => text == "youarel" || text == "P4suta/youarel",
        Value::Array(entries) => entries.iter().any(has_repository),
        Value::Object(fields) => fields.values().any(has_repository),
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
    }
}

struct Api<'a> {
    root: &'a Path,
    token: String,
}

fn quote(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

fn metadata_endpoint(path: &str) -> bool {
    let endpoint = path.split('?').next().unwrap_or_default();
    matches!(
        endpoint,
        "/v3/integrations"
            | "/v3/integrations/integration/options"
            | "/v3/workplace/service_accounts"
            | "/v3/configs/config/syncs"
            | "/v3/configs/config/syncs/sync"
    ) || endpoint
        .strip_prefix("/v3/workplace/service_accounts/service_account/")
        .and_then(|suffix| suffix.strip_suffix("/identities"))
        .is_some_and(|slug| youarel_release::manifest::Name::parse(slug).is_some())
}

impl<'a> Api<'a> {
    fn new(root: &'a Path, scope: &str) -> Result<Self> {
        ensure!(
            Path::new(scope).is_absolute(),
            "select an absolute local Doppler authentication scope"
        );
        let output = tool::output(
            Tool::Doppler.command(root).args([
                "configure",
                "get",
                "token",
                "--plain",
                "--scope",
                scope,
            ]),
            "read the local Doppler authentication input",
        )?;
        ensure!(
            output.status.success(),
            "Doppler authentication is unavailable for the selected local scope"
        );
        let token = String::from_utf8(output.stdout)
            .context("Doppler authentication is not UTF-8")?
            .trim()
            .to_owned();
        ensure!(
            token.starts_with("dp.")
                && token
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')),
            "invalid Doppler authentication input"
        );
        Ok(Self { root, token })
    }

    fn request(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value> {
        ensure!(
            ["GET", "POST"].contains(&method) && metadata_endpoint(path),
            "unsupported access operation"
        );
        let mut config = format!(
            "url = \"{}{}\"\nrequest = \"{method}\"\nheader = \"Authorization: Bearer {}\"\n",
            API,
            quote(path),
            self.token
        );
        if let Some(body) = body {
            writeln!(
                config,
                "header = \"Content-Type: application/json\"\ndata = \"{}\"",
                quote(&serde_json::to_string(body)?)
            )?;
        }
        let mut child = Tool::Curl
            .command(self.root)
            .args([
                "--disable",
                "--http1.1",
                "--config",
                "-",
                "--silent",
                "--show-error",
                "--fail-with-body",
                "--proto",
                "=https",
                "--max-time",
                REQUEST_TIMEOUT,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("start the Doppler API transport")?;
        let mut input = child.stdin.take().context("open the private API input")?;
        input
            .write_all(config.as_bytes())
            .context("write the private API input")?;
        drop(input);
        let output = child
            .wait_with_output()
            .context("wait for the Doppler API")?;
        ensure!(
            output.status.success(),
            "Doppler {method} {path} failed with {}: {} {}; inspect provider state before retrying a write",
            output.status,
            String::from_utf8_lossy(&output.stderr).replace(&self.token, "[redacted]"),
            String::from_utf8_lossy(&output.stdout).replace(&self.token, "[redacted]")
        );
        serde_json::from_slice(&output.stdout)
            .context("Doppler returned an invalid metadata response")
    }

    fn list(&self, path: &str, field: &str) -> Result<Vec<Value>> {
        let mut found = Vec::new();
        for page in 1..=MAX_PAGES {
            let separator = if path.contains('?') { '&' } else { '?' };
            let value = self.request(
                "GET",
                &format!("{path}{separator}page={page}&per_page={PAGE_SIZE}"),
                None,
            )?;
            let entries = value
                .get(field)
                .and_then(Value::as_array)
                .with_context(|| format!("Doppler returned no {field} metadata"))?;
            found.extend(entries.iter().cloned());
            if entries.len() < PAGE_SIZE {
                return Ok(found);
            }
        }
        anyhow::bail!("Doppler metadata pagination did not finish")
    }
}

fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .with_context(|| format!("Doppler metadata lacks {key}"))
}

fn syncs(api: &Api<'_>, prepare: bool) -> Result<()> {
    let value = api.request("GET", "/v3/integrations", None)?;
    let integrations = value
        .get("integrations")
        .and_then(Value::as_array)
        .context("Doppler returned no integrations")?;
    let count: usize = integrations
        .iter()
        .map(|integration| {
            integration
                .get("syncs")
                .and_then(Value::as_array)
                .map_or(0, Vec::len)
        })
        .sum();
    println!("Existing native syncs: {count}/{SYNC_LIMIT}.");
    ensure!(
        !prepare || count.saturating_add(PURPOSES.len()) <= SYNC_LIMIT,
        "the final sync layout exceeds the conservative Team plan limit"
    );
    let matching: Vec<&Value> = integrations
        .iter()
        .filter(|integration| {
            integration.get("name").and_then(Value::as_str) == Some("P4suta")
                && integration.get("type").and_then(Value::as_str) == Some("github-actions")
        })
        .collect();
    ensure!(
        matching.len() == 1,
        "the GitHub integration is missing or ambiguous"
    );
    let integration = matching
        .first()
        .context("the GitHub integration is missing")?;
    ensure!(
        integration.get("enabled").and_then(Value::as_bool) == Some(true),
        "the GitHub integration is disabled"
    );
    let existing = integration
        .get("syncs")
        .and_then(Value::as_array)
        .context("integration sync metadata is missing")?;
    let slug = text(integration, "slug")?;
    ensure!(
        youarel_release::manifest::Name::parse(slug).is_some(),
        "unsafe integration identifier"
    );
    let options = api.request(
        "GET",
        &format!("/v3/integrations/integration/options?integration={slug}"),
        None,
    )?;
    ensure!(
        has_repository(&options),
        "the GitHub integration cannot reach youarel"
    );
    if prepare {
        for purpose in PURPOSES {
            prepare_sync(api, slug, existing, purpose)?;
        }
    } else {
        for sync in existing
            .iter()
            .filter(|sync| sync.get("project").and_then(Value::as_str) == Some("youarel"))
        {
            println!(
                "Sync {}: config {}, enabled {}, status {}",
                text(sync, "slug")?,
                text(sync, "config")?,
                sync.get("enabled").unwrap_or(&Value::Null),
                sync.get("status").unwrap_or(&Value::Null)
            );
        }
    }
    Ok(())
}

fn prepare_sync(api: &Api<'_>, integration: &str, existing: &[Value], purpose: &str) -> Result<()> {
    let matching = existing.iter().any(|sync| {
        sync.get("project").and_then(Value::as_str) == Some("youarel")
            && sync.get("config").and_then(Value::as_str) == Some(purpose)
    });
    ensure!(
        !matching,
        "youarel/{purpose} already has a sync; inspect its destination and status instead of creating another writer"
    );
    let request = json!({"integration": integration, "import_option":"none", "await_initial_sync":true, "data":sync_data(purpose)});
    let value = api.request(
        "POST",
        &format!("/v3/configs/config/syncs?project=youarel&config={purpose}"),
        Some(&request),
    )?;
    let sync = value
        .get("sync")
        .context("Doppler did not identify the sync")?;
    ensure!(
        sync.get("project").and_then(Value::as_str) == Some("youarel")
            && sync.get("config").and_then(Value::as_str) == Some(purpose),
        "the sync identifies another source config"
    );
    println!(
        "Created sync {} from youarel/{purpose} to P4suta/youarel environment {}.",
        text(sync, "slug")?,
        environment(purpose)
    );
    Ok(())
}

pub(super) fn run(root: &Path, operation: &str, scope: &str) -> Result<()> {
    ensure!(
        ["inventory", "sync-inventory", "sync-prepare"].contains(&operation),
        "unknown access operation"
    );
    let api = Api::new(root, scope)?;
    if operation != "inventory" {
        return syncs(&api, operation == "sync-prepare");
    }
    let accounts = api.list("/v3/workplace/service_accounts", "service_accounts")?;
    let mut identity_count = 0_usize;
    for account in &accounts {
        let slug = text(account, "slug")?;
        ensure!(
            youarel_release::manifest::Name::parse(slug).is_some(),
            "unsafe service account identifier"
        );
        identity_count = identity_count.saturating_add(
            api.list(
                &format!("/v3/workplace/service_accounts/service_account/{slug}/identities"),
                "identities",
            )?
            .len(),
        );
    }
    println!(
        "Doppler service accounts: {}; OIDC identities: {identity_count}.",
        accounts.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{PURPOSES, metadata_endpoint, quote, sync_data};

    #[test]
    fn access_administration_cannot_fetch_secret_values() {
        assert!(metadata_endpoint(
            "/v3/configs/config/syncs?project=youarel"
        ));
        assert!(!metadata_endpoint("/v3/configs/config/secrets"));
        assert!(!metadata_endpoint(
            "/v3/integrations/../configs/config/secrets"
        ));
        assert!(!metadata_endpoint("/v3/configs/config/syncs/../../secrets"));
        assert!(!metadata_endpoint("https://other.example/v3/integrations"));
    }

    #[test]
    fn credential_syncs_target_separate_environments_in_the_product_repository() {
        let mut destinations = std::collections::BTreeSet::new();
        for purpose in PURPOSES {
            let data = sync_data(purpose);
            assert_eq!(data.get("repo_name").unwrap(), "youarel");
            assert_eq!(data.get("sync_target").unwrap(), "repo");
            assert_eq!(data.get("feature").unwrap(), "actions");
            assert!(
                destinations.insert(
                    data.get("environment_name")
                        .unwrap()
                        .as_str()
                        .unwrap()
                        .to_owned()
                )
            );
        }
    }

    #[test]
    fn transport_configuration_quotes_json_without_adding_options() {
        assert_eq!(quote("a\"\nb\\c\r"), "a\\\"\\nb\\\\c\\r");
    }
}
