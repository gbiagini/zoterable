use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct Config {
    /// Numeric user ID shown at https://www.zotero.org/settings/keys
    pub zotero_user_id: String,
    /// API key with library read access
    pub zotero_api_key: String,
    /// Numeric IDs of group libraries to sync in addition to the personal
    /// library. The API key needs group read access for these.
    #[serde(default)]
    pub zotero_group_ids: Vec<String>,
}

impl Config {
    /// API path prefixes of all libraries to sync ("users/…" and "groups/…").
    pub fn libraries(&self) -> Vec<String> {
        let mut libraries = vec![self.user_library()];
        libraries.extend(self.zotero_group_ids.iter().map(|id| format!("groups/{id}")));
        libraries
    }

    pub fn user_library(&self) -> String {
        format!("users/{}", self.zotero_user_id)
    }
}

pub fn config_dir() -> Result<PathBuf> {
    let dir = dirs::config_dir()
        .context("could not determine the user config directory")?
        .join("zoterable");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.toml"))
}

pub fn device_token_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("remarkable-device-token"))
}

pub fn state_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("state.json"))
}

pub fn load() -> Result<Config> {
    let path = config_path()?;
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("cannot read {} — run `zoterable init` first", path.display()))?;
    let config: Config =
        toml::from_str(&raw).with_context(|| format!("invalid config at {}", path.display()))?;
    if config.zotero_user_id.is_empty() || config.zotero_api_key.is_empty() {
        bail!(
            "fill in zotero_user_id and zotero_api_key in {}",
            path.display()
        );
    }
    Ok(config)
}

const TEMPLATE: &str = "\
# Your numeric user ID and an API key with library read access,
# both from https://www.zotero.org/settings/keys
zotero_user_id = \"\"
zotero_api_key = \"\"

# Optional: numeric IDs of group libraries to sync too (the number in the
# group's URL, https://www.zotero.org/groups/<id>/<name>). The API key must
# have group read access enabled.
zotero_group_ids = []
";

pub fn init() -> Result<()> {
    let path = config_path()?;
    if path.exists() {
        println!("Config already exists at {}", path.display());
    } else {
        fs::write(&path, TEMPLATE)?;
        println!("Wrote config template to {}", path.display());
    }
    println!();
    println!("Next steps:");
    println!("  1. Create an API key at https://www.zotero.org/settings/keys and fill in the config.");
    println!("  2. Get a one-time code at https://my.remarkable.com/device/browser/connect");
    println!("     and run `zoterable pair <code>` (codes expire after a few minutes).");
    println!("  3. Run `zoterable sync`.");
    Ok(())
}
