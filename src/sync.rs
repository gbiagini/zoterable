use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::config;
use crate::remarkable::Remarkable;
use crate::zotero::{Item, Zotero};

#[derive(Clone, Default, Serialize, Deserialize)]
struct LibraryState {
    /// Zotero library version at the last completed sync; only items modified
    /// after this are fetched.
    #[serde(default)]
    last_library_version: u64,
    /// Zotero attachment key -> item version at last successful upload.
    #[serde(default)]
    synced: HashMap<String, u64>,
}

#[derive(Default, Serialize, Deserialize)]
struct State {
    /// Per-library sync state, keyed by API prefix ("users/…" or "groups/…").
    #[serde(default)]
    libraries: HashMap<String, LibraryState>,
    // Legacy fields from the single-library format; migrated into `libraries`
    // (under the personal library) on load.
    #[serde(default, skip_serializing_if = "is_zero")]
    last_library_version: u64,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    synced: HashMap<String, u64>,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

impl State {
    fn load(path: &Path, user_library: &str) -> Self {
        let mut state: State = fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        if state.last_library_version != 0 || !state.synced.is_empty() {
            let lib = state.libraries.entry(user_library.to_string()).or_default();
            lib.last_library_version = lib.last_library_version.max(state.last_library_version);
            lib.synced.extend(std::mem::take(&mut state.synced));
            state.last_library_version = 0;
        }
        state
    }

    fn save(&self, path: &Path) -> Result<()> {
        fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}

pub fn run(dry_run: bool) -> Result<()> {
    let cfg = config::load()?;
    let zotero = Zotero::new(&cfg.zotero_api_key);

    let state_path = config::state_path()?;
    let mut state = State::load(&state_path, &cfg.user_library());

    let mut remarkable: Option<Remarkable> = None;
    let mut failures = 0usize;

    for library in cfg.libraries() {
        let mut lib = state.libraries.get(&library).cloned().unwrap_or_default();
        println!(
            "[{library}] fetching PDF attachments changed since library version {}…",
            lib.last_library_version
        );
        let (attachments, library_version) =
            zotero.pdf_attachments(&library, lib.last_library_version)?;

        // Only never-seen attachments are uploaded. Re-uploading a known key
        // would create a duplicate document on the reMarkable (the upload
        // endpoint has no replace semantics), so metadata-only edits just
        // refresh the record.
        let (new, updated): (Vec<&Item>, Vec<&Item>) = attachments
            .iter()
            .partition(|item| !lib.synced.contains_key(&item.key));
        println!(
            "[{library}] {} changed attachment(s): {} new, {} already on the tablet.",
            attachments.len(),
            new.len(),
            updated.len()
        );

        if dry_run {
            for item in new {
                println!("would upload: {}", display_name(&zotero, &library, item));
            }
            continue;
        }

        for item in &updated {
            lib.synced.insert(item.key.clone(), item.version);
        }

        let mut lib_failures = 0usize;
        for item in new {
            let remarkable = match &remarkable {
                Some(r) => r,
                None => remarkable.insert(Remarkable::connect()?),
            };
            let name = display_name(&zotero, &library, item);
            let result = zotero
                .download(&library, &item.key)
                .and_then(|bytes| remarkable.upload_pdf(&name, bytes));
            match result {
                Ok(()) => {
                    println!("uploaded: {name}");
                    lib.synced.insert(item.key.clone(), item.version);
                    state.libraries.insert(library.clone(), lib.clone());
                    state.save(&state_path)?;
                }
                Err(err) => {
                    lib_failures += 1;
                    eprintln!("FAILED: {name}: {err:#}");
                }
            }
        }

        // Only advance the version when everything uploaded, so failed items
        // are picked up again on the next run.
        if lib_failures == 0 {
            lib.last_library_version = library_version;
        }
        failures += lib_failures;
        state.libraries.insert(library.clone(), lib);
        state.save(&state_path)?;
    }

    if failures > 0 {
        bail!("{failures} upload(s) failed — they will be retried on the next sync");
    }
    Ok(())
}

/// Record every PDF attachment currently in the configured libraries as
/// already synced, without uploading anything. After this, `sync` only sends
/// future additions.
pub fn baseline() -> Result<()> {
    let cfg = config::load()?;
    let zotero = Zotero::new(&cfg.zotero_api_key);

    let state_path = config::state_path()?;
    let mut state = State::load(&state_path, &cfg.user_library());

    for library in cfg.libraries() {
        println!("[{library}] fetching all PDF attachments…");
        let (attachments, library_version) = zotero.pdf_attachments(&library, 0)?;
        let lib = state.libraries.entry(library.clone()).or_default();
        let already = lib.synced.len();
        for item in &attachments {
            lib.synced.insert(item.key.clone(), item.version);
        }
        lib.last_library_version = library_version;
        println!(
            "[{library}] marked {} attachment(s) as synced ({} were already recorded).",
            attachments.len(),
            already
        );
    }
    state.save(&state_path)?;
    println!("Future `zoterable sync` runs will only upload newly added PDFs.");
    Ok(())
}

/// Build "Author - Year - Title" from the attachment's parent item, falling
/// back to the attachment's own filename when there is no usable parent.
fn display_name(zotero: &Zotero, library: &str, item: &Item) -> String {
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
    let Ok(parent) = zotero.item(library, parent_key) else {
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
