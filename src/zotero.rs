use anyhow::{Context, Result};
use reqwest::blocking::{Client, RequestBuilder};
use serde::Deserialize;

const API: &str = "https://api.zotero.org";
const PAGE_SIZE: usize = 100;

pub struct Zotero {
    client: Client,
    user_id: String,
    api_key: String,
}

#[derive(Deserialize)]
pub struct Item {
    pub key: String,
    pub version: u64,
    pub data: ItemData,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemData {
    #[serde(default)]
    pub content_type: Option<String>,
    #[serde(default)]
    pub link_mode: Option<String>,
    #[serde(default)]
    pub parent_item: Option<String>,
    #[serde(default)]
    pub filename: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub creators: Vec<Creator>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Creator {
    #[serde(default)]
    pub last_name: Option<String>,
    /// Single-field creator names (e.g. institutions).
    #[serde(default)]
    pub name: Option<String>,
}

impl Zotero {
    pub fn new(user_id: &str, api_key: &str) -> Self {
        Self {
            client: Client::new(),
            user_id: user_id.to_string(),
            api_key: api_key.to_string(),
        }
    }

    fn get(&self, path_and_query: &str) -> RequestBuilder {
        self.client
            .get(format!("{API}/users/{}/{path_and_query}", self.user_id))
            .header("Zotero-API-Key", &self.api_key)
            .header("Zotero-API-Version", "3")
    }

    /// PDF attachments stored in the library that were added or modified after
    /// library version `since` (pass 0 for everything). Returns the items and
    /// the current library version, for use as the next `since`. Linked-file
    /// attachments are skipped because their content lives outside Zotero
    /// storage.
    pub fn pdf_attachments(&self, since: u64) -> Result<(Vec<Item>, u64)> {
        let mut items: Vec<Item> = Vec::new();
        let mut library_version = since;
        let mut start = 0;
        loop {
            let response = self
                .get(&format!(
                    "items?itemType=attachment&since={since}&limit={PAGE_SIZE}&start={start}"
                ))
                .send()?
                .error_for_status()
                .context("Zotero item listing failed — check zotero_user_id and zotero_api_key")?;
            if let Some(version) = response
                .headers()
                .get("Last-Modified-Version")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse().ok())
            {
                library_version = version;
            }
            let batch: Vec<Item> = response.json()?;
            let done = batch.len() < PAGE_SIZE;
            start += batch.len();
            items.extend(batch);
            if done {
                break;
            }
        }
        items.retain(|item| {
            item.data.content_type.as_deref() == Some("application/pdf")
                && matches!(
                    item.data.link_mode.as_deref(),
                    Some("imported_file" | "imported_url")
                )
        });
        Ok((items, library_version))
    }

    pub fn item(&self, key: &str) -> Result<Item> {
        Ok(self
            .get(&format!("items/{key}"))
            .send()?
            .error_for_status()
            .with_context(|| format!("could not fetch Zotero item {key}"))?
            .json()?)
    }

    /// Download an attachment's file content (follows the redirect to storage).
    pub fn download(&self, key: &str) -> Result<Vec<u8>> {
        let response = self
            .get(&format!("items/{key}/file"))
            .send()?
            .error_for_status()
            .with_context(|| format!("could not download attachment {key}"))?;
        Ok(response.bytes()?.to_vec())
    }
}
