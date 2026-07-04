use std::collections::HashMap;
use std::fs;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::config;
use crate::remarkable::Remarkable;
use crate::zotero::{Item, Zotero};

#[derive(Default, Serialize, Deserialize)]
struct State {
    /// Zotero library version at the last completed sync; only items modified
    /// after this are fetched.
    #[serde(default)]
    last_library_version: u64,
    /// Zotero attachment key -> item version at last successful upload.
    synced: HashMap<String, u64>,
}

impl State {
    fn load(path: &std::path::Path) -> Self {
        fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    fn save(&self, path: &std::path::Path) -> Result<()> {
        fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}

pub fn run(dry_run: bool) -> Result<()> {
    let cfg = config::load()?;
    let zotero = Zotero::new(&cfg.zotero_user_id, &cfg.zotero_api_key);

    let state_path = config::state_path()?;
    let mut state = State::load(&state_path);

    println!(
        "Fetching Zotero PDF attachments changed since library version {}…",
        state.last_library_version
    );
    let (attachments, library_version) = zotero.pdf_attachments(state.last_library_version)?;

    // Only never-seen attachments are uploaded. Re-uploading a known key would
    // create a duplicate document on the reMarkable (the upload endpoint has
    // no replace semantics), so metadata-only edits just refresh the record.
    let (new, updated): (Vec<&Item>, Vec<&Item>) = attachments
        .iter()
        .partition(|item| !state.synced.contains_key(&item.key));
    println!(
        "{} changed attachment(s): {} new, {} already on the tablet.",
        attachments.len(),
        new.len(),
        updated.len()
    );

    if dry_run {
        for item in new {
            println!("would upload: {}", display_name(&zotero, item));
        }
        return Ok(());
    }

    for item in &updated {
        state.synced.insert(item.key.clone(), item.version);
    }

    let mut failures = 0usize;
    if !new.is_empty() {
        let remarkable = Remarkable::connect()?;
        for item in new {
            let name = display_name(&zotero, item);
            let result = zotero
                .download(&item.key)
                .and_then(|bytes| remarkable.upload_pdf(&name, bytes));
            match result {
                Ok(()) => {
                    println!("uploaded: {name}");
                    state.synced.insert(item.key.clone(), item.version);
                    state.save(&state_path)?;
                }
                Err(err) => {
                    failures += 1;
                    eprintln!("FAILED: {name}: {err:#}");
                }
            }
        }
    }

    if failures > 0 {
        // Leave last_library_version alone so the failed items are picked up
        // again on the next run.
        state.save(&state_path)?;
        bail!("{failures} upload(s) failed — they will be retried on the next sync");
    }
    state.last_library_version = library_version;
    state.save(&state_path)?;
    Ok(())
}

/// Record every PDF attachment currently in the library as already synced,
/// without uploading anything. After this, `sync` only sends future additions.
pub fn baseline() -> Result<()> {
    let cfg = config::load()?;
    let zotero = Zotero::new(&cfg.zotero_user_id, &cfg.zotero_api_key);

    let state_path = config::state_path()?;
    let mut state = State::load(&state_path);

    println!("Fetching all Zotero PDF attachments…");
    let (attachments, library_version) = zotero.pdf_attachments(0)?;
    let already = state.synced.len();
    for item in &attachments {
        state.synced.insert(item.key.clone(), item.version);
    }
    state.last_library_version = library_version;
    state.save(&state_path)?;
    println!(
        "Marked {} attachment(s) as synced ({} were already recorded). \
         Future `zoterable sync` runs will only upload newly added PDFs.",
        attachments.len(),
        already
    );
    Ok(())
}

/// Build "Author - Year - Title" from the attachment's parent item, falling
/// back to the attachment's own filename when there is no usable parent.
fn display_name(zotero: &Zotero, item: &Item) -> String {
    let fallback = item
        .data
        .filename
        .clone()
        .or_else(|| item.data.title.clone())
        .unwrap_or_else(|| item.key.clone());
    let fallback = sanitize(fallback.trim_end_matches(".pdf").trim_end_matches(".PDF"));

    let Some(parent_key) = &item.data.parent_item else {
        return fallback;
    };
    let Ok(parent) = zotero.item(parent_key) else {
        return fallback;
    };

    let mut parts: Vec<String> = Vec::new();
    let authors: Vec<&str> = parent
        .data
        .creators
        .iter()
        .filter_map(|c| c.last_name.as_deref().or(c.name.as_deref()))
        .collect();
    match authors[..] {
        [] => {}
        [one] => parts.push(one.to_string()),
        [first, second] => parts.push(format!("{first} & {second}")),
        [first, ..] => parts.push(format!("{first} et al.")),
    }
    if let Some(year) = parent.data.date.as_deref().and_then(extract_year) {
        parts.push(year);
    }
    if let Some(title) = parent.data.title.as_deref().filter(|t| !t.is_empty()) {
        parts.push(title.to_string());
    }

    if parts.is_empty() {
        fallback
    } else {
        sanitize(&parts.join(" - "))
    }
}

/// First run of four consecutive digits, e.g. "2023" from "2023-05-01".
fn extract_year(date: &str) -> Option<String> {
    let mut run = String::new();
    for c in date.chars() {
        if c.is_ascii_digit() {
            run.push(c);
            if run.len() == 4 {
                return Some(run);
            }
        } else {
            run.clear();
        }
    }
    None
}

fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let mut out = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if out.len() > 128 {
        let mut cut = 128;
        while !out.is_char_boundary(cut) {
            cut -= 1;
        }
        out.truncate(cut);
    }
    out
}
