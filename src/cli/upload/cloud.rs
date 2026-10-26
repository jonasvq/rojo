use std::{
    io::Read,
    time::{Duration, Instant},
};

use anyhow::{bail, ensure, Context};
use reqwest::{
    blocking::{Client, RequestBuilder},
    header::{HeaderMap, HeaderValue, ACCEPT, CONTENT_TYPE},
    redirect::Policy,
};
use serde_json::Value;

const RESPONSE_LIMIT: u64 = 1024 * 1024;

pub(super) enum Target {
    Place { universe_id: u64, asset_id: u64 },
    Model { asset_id: u64 },
}

pub(super) struct Cloud {
    client: Client,
    base: String,
    key: String,
    deadline: Duration,
    delay: Duration,
}

impl Cloud {
    pub(super) fn new(key: &str) -> anyhow::Result<Self> {
        let client = client_builder(key)?.build()?;
        Ok(Self {
            client,
            base: "https://apis.roblox.com".into(),
            key: key.into(),
            deadline: Duration::from_secs(120),
            delay: Duration::from_secs(1),
        })
    }

    pub(super) fn upload(&self, bytes: Vec<u8>, target: Target) -> anyhow::Result<()> {
        self.upload_inner(bytes, target)
            .map_err(|error| anyhow::anyhow!(self.redact(&format!("{error:#}"))))
    }

    fn upload_inner(&self, bytes: Vec<u8>, target: Target) -> anyhow::Result<()> {
        match target {
            Target::Place {
                universe_id,
                asset_id,
            } => {
                let url = format!("{}/universes/v1/{universe_id}/places/{asset_id}/versions?versionType=Published", self.base);
                let response = self.json(
                    self.client
                        .post(url)
                        .header(CONTENT_TYPE, "application/octet-stream")
                        .body(bytes),
                )?;
                let version = response
                    .get("versionNumber")
                    .and_then(Value::as_u64)
                    .filter(|version| *version > 0)
                    .context("Place publishing response has no valid versionNumber")?;
                println!("Published place {asset_id}, version {version}");
            }
            Target::Model { asset_id } => {
                let (boundary, body) = multipart(asset_id, &bytes);
                let url = format!("{}/assets/v1/assets/{asset_id}", self.base);
                let started = Instant::now();
                let mut operation = self.json(
                    self.client
                        .patch(url)
                        .timeout(self.deadline.min(Duration::from_secs(60)))
                        .header(
                            CONTENT_TYPE,
                            format!("multipart/form-data; boundary={boundary}"),
                        )
                        .body(body),
                )?;
                let path = operation
                    .get("path")
                    .and_then(Value::as_str)
                    .context("Asset update response has no operation path")?
                    .to_owned();
                ensure!(
                    valid_operation_path(&path),
                    "Asset update returned an invalid operation path"
                );
                let mut delay = self.delay;
                loop {
                    ensure!(
                        operation.is_object(),
                        "Asset operation {path} is not an object"
                    );
                    if operation.get("error").is_some_and(|value| !value.is_null()) {
                        bail!("Asset update {path} failed: {}", operation["error"]);
                    }
                    match operation.get("done") {
                        Some(Value::Bool(true)) => {
                            let response = operation
                                .get("response")
                                .context("Completed asset operation has no response")?;
                            ensure!(
                                response.is_object(),
                                "Completed asset response is not an object"
                            );
                            let id = match response.get("assetId") {
                                None => None,
                                Some(Value::String(id)) => Some(id.clone()),
                                Some(value) => Some(
                                    value
                                        .as_u64()
                                        .context("Invalid completed asset ID")?
                                        .to_string(),
                                ),
                            };
                            let resource = response
                                .get("path")
                                .map(|value| {
                                    value
                                        .as_str()
                                        .context("Invalid completed asset resource path")
                                })
                                .transpose()?;
                            ensure!(
                                id.as_deref() == Some(&asset_id.to_string())
                                    || resource == Some(&format!("assets/{asset_id}")),
                                "Completed asset operation targets a different or missing asset ID"
                            );
                            // If both identifiers are present, neither may name
                            // another target even if the other one matches.
                            ensure!(
                                id.as_deref().is_none_or(|id| id == asset_id.to_string())
                                    && resource
                                        .is_none_or(|path| path == format!("assets/{asset_id}")),
                                "Completed asset operation has conflicting asset identifiers"
                            );
                            println!("Updated model {asset_id}");
                            break;
                        }
                        None | Some(Value::Bool(false)) => {}
                        _ => bail!("Asset operation {path} has an invalid done field"),
                    }
                    let remaining = self
                        .deadline
                        .checked_sub(started.elapsed())
                        .context(format!("Timed out waiting for asset operation {path}"))?;
                    ensure!(
                        !remaining.is_zero(),
                        "Timed out waiting for asset operation {path}"
                    );
                    std::thread::sleep(delay.min(remaining));
                    let remaining = self
                        .deadline
                        .checked_sub(started.elapsed())
                        .context(format!("Timed out waiting for asset operation {path}"))?;
                    operation = self
                        .json(
                            self.client
                                .get(format!("{}/assets/v1/{path}", self.base))
                                .timeout(remaining.min(Duration::from_secs(60))),
                        )
                        .with_context(|| format!("Reading asset operation {path}"))?;
                    ensure!(
                        operation.get("path").and_then(Value::as_str) == Some(path.as_str()),
                        "Asset operation response changed or omitted its path"
                    );
                    delay = (delay * 2).min(Duration::from_secs(4));
                }
            }
        }
        Ok(())
    }

    fn json(&self, request: RequestBuilder) -> anyhow::Result<Value> {
        let response = request.send().context("Open Cloud request failed")?;
        let status = response.status();
        let mut bytes = Vec::new();
        response
            .take(RESPONSE_LIMIT + 1)
            .read_to_end(&mut bytes)
            .context("Could not read Open Cloud response")?;
        ensure!(
            bytes.len() as u64 <= RESPONSE_LIMIT,
            "Open Cloud response exceeded 1 MiB (HTTP {status})"
        );
        if !status.is_success() {
            let text = self.redact(&String::from_utf8_lossy(&bytes));
            bail!(
                "Open Cloud returned HTTP {status}: {}",
                text.chars().take(8192).collect::<String>()
            );
        }
        serde_json::from_slice(&bytes).context("Open Cloud returned invalid JSON")
    }

    fn redact(&self, text: &str) -> String {
        let escaped = serde_json::to_string(&self.key).unwrap();
        text.replace(&self.key, "[REDACTED]")
            .replace(&escaped[1..escaped.len() - 1], "[REDACTED]")
    }
}

fn client_builder(key: &str) -> anyhow::Result<reqwest::blocking::ClientBuilder> {
    ensure!(!key.trim().is_empty(), "--api_key must not be empty");
    let mut header = HeaderValue::from_str(key).context("Invalid --api_key header value")?;
    header.set_sensitive(true);
    let mut headers = HeaderMap::new();
    headers.insert("x-api-key", header);
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    Ok(Client::builder()
        .default_headers(headers)
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(60)))
}

fn valid_operation_path(path: &str) -> bool {
    path.strip_prefix("operations/").is_some_and(|id| {
        !id.is_empty()
            && id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    })
}

fn multipart(asset_id: u64, bytes: &[u8]) -> (String, Vec<u8>) {
    let json = serde_json::json!({"assetId":asset_id.to_string()}).to_string();
    let boundary = loop {
        let boundary = format!("rojo-{}", uuid::Uuid::new_v4().simple());
        if !bytes
            .windows(boundary.len())
            .any(|window| window == boundary.as_bytes())
            && !json.contains(&boundary)
        {
            break boundary;
        }
    };
    let prefix = format!("--{boundary}\r\nContent-Disposition: form-data; name=\"request\"\r\nContent-Type: application/json\r\n\r\n{json}\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"fileContent\"; filename=\"model.rbxm\"\r\nContent-Type: model/x-rbxm\r\n\r\n");
    let mut body = prefix.into_bytes();
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (boundary, body)
}

#[cfg(test)]
mod tests;
