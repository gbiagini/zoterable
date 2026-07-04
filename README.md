# zoterable

Syncs PDF attachments from your Zotero library to your reMarkable tablet, renamed
to `Author - Year - Title` from the Zotero item metadata.

It talks to the Zotero Web API and the reMarkable cloud, so it works with the
desktop app closed and no cable plugged in. Sync is incremental: only items
changed since the last run are even fetched (via the Zotero `since` parameter),
and each attachment is uploaded exactly once — metadata edits to an
already-synced item never create a duplicate on the tablet.

## Setup

```sh
cargo install --path .
zoterable init
```

Then:

1. Create an API key (library read access) at <https://www.zotero.org/settings/keys>
   and fill in `zotero_user_id` and `zotero_api_key` in the config file that
   `init` created.
2. Get a one-time pairing code at <https://my.remarkable.com/device/browser/connect>
   and run `zoterable pair <code>` (codes expire after a few minutes).

3. **If you have an existing library you don't want dumped onto the tablet**,
   run `zoterable baseline` once. It marks every PDF currently in Zotero as
   already synced, so only papers added afterwards get uploaded.

## Usage

```sh
zoterable baseline        # one-time: skip everything already in the library
zoterable sync            # upload newly added PDFs
zoterable sync --dry-run  # show what would be uploaded
```

Uploads land in the root folder of the reMarkable (the simple cloud upload
endpoint cannot target subfolders). Linked-file attachments are skipped, since
their content is not in Zotero storage.

Config, tokens, and sync state live in `~/Library/Application Support/zoterable/`
(macOS) or `~/.config/zoterable/` (Linux). Delete `state.json` to force a full
re-upload.

## Running automatically

On macOS, a launchd agent runs the sync on a schedule. Save this as
`~/Library/LaunchAgents/com.zoterable.sync.plist` (adjust the binary path to
`which zoterable`):

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>com.zoterable.sync</string>
  <key>ProgramArguments</key>
  <array>
    <string>/Users/YOU/.cargo/bin/zoterable</string>
    <string>sync</string>
  </array>
  <key>StartInterval</key><integer>3600</integer>
  <key>StandardOutPath</key><string>/tmp/zoterable.log</string>
  <key>StandardErrorPath</key><string>/tmp/zoterable.log</string>
</dict>
</plist>
```

Then load it with:

```sh
launchctl load ~/Library/LaunchAgents/com.zoterable.sync.plist
```
