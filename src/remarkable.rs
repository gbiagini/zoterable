use std::fs;

use anyhow::{Context, Result};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use reqwest::blocking::Client;

use crate::config;

const AUTH_HOST: &str = "https://webapp-prod.cloud.remarkable.engineering";
const UPLOAD_HOST: &str = "https://internal.cloud.remarkable.com";

/// Register this machine with the reMarkable cloud and store the device token.
pub fn pair(code: &str) -> Result<()> {
    let response = Client::new()
        .post(format!("{AUTH_HOST}/token/json/2/device/new"))
        .bearer_auth("")
        .json(&serde_json::json!({
            "code": code,
            "deviceDesc": "browser-chrome",
            "deviceID": uuid::Uuid::new_v4().to_string(),
        }))
        .send()?
        .error_for_status()
        .context("device registration failed — one-time codes expire quickly, get a fresh one")?;
    let token = response.text()?;
    let path = config::device_token_path()?;
    fs::write(&path, &token)?;
    println!("Paired with the reMarkable cloud (token stored in {}).", path.display());
    Ok(())
}

pub struct Remarkable {
    client: Client,
    session_token: String,
}

impl Remarkable {
    /// Exchange the stored device token for a fresh session token.
    pub fn connect() -> Result<Self> {
        let path = config::device_token_path()?;
        let device_token = fs::read_to_string(&path).with_context(|| {
            format!("cannot read {} — run `zoterable pair <code>` first", path.display())
        })?;
        let client = Client::new();
        let session_token = client
            .post(format!("{AUTH_HOST}/token/json/2/user/new"))
            .bearer_auth(device_token.trim())
            .send()?
            .error_for_status()
            .context("could not refresh the reMarkable session token — try re-pairing")?
            .text()?;
        Ok(Self { client, session_token })
    }

    /// Upload a PDF to the root folder of the reMarkable cloud.
    ///
    /// Uses the simple `doc/v2/files` upload endpoint (the one the "Read on
    /// reMarkable" browser extension uses), which cannot target a subfolder.
    pub fn upload_pdf(&self, visible_name: &str, bytes: Vec<u8>) -> Result<()> {
        let meta = BASE64.encode(serde_json::json!({ "file_name": visible_name }).to_string());
        self.client
            .post(format!("{UPLOAD_HOST}/doc/v2/files"))
            .bearer_auth(&self.session_token)
            .header("content-type", "application/pdf")
            .header("rm-meta", meta)
            .header("rm-source", "RoR-Browser")
            .body(bytes)
            .send()?
            .error_for_status()
            .with_context(|| format!("upload of {visible_name:?} failed"))?;
        Ok(())
    }
}
