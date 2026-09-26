# Isolated media lab

This Compose project runs pinned Sonarr, Radarr, and Plex images with separate
named volumes and loopback ports. It never mounts production media. Python 3,
Docker Engine with Compose v2, and a built yarr binary are required. Run inside
Linux or WSL, on the same machine as the Docker daemon.

```sh
cargo build --bin yarr
python3 tests/media-lab/lab.py up
python3 tests/media-lab/lab.py status
python3 -m unittest discover -s tests/media-lab -p 'test_*.py'
python3 tests/media-lab/lab.py test --binary target/debug/yarr
```

The test runner verifies Compose ownership, healthy containers, exclusive
loopback bindings, volume ownership labels, and isolated networking before any API
calls. Lifecycle commands check existing resources and force the lab's project
name. Remote Docker contexts are refused. Credentials are read into memory from the test containers;
inherited yarr configuration and proxy variables are excluded. Reports contain
versions, image IDs, binary hash, source revision, and outcomes, never credentials. The default report is
`.cache/media-lab/report.json`; use `--report PATH` to save elsewhere.

Sonarr and Radarr checks cover status, quality profile and custom format
create/read/update/delete, tag create/delete, reversible naming configuration,
and invalid parameter rejection. Live stdio MCP sessions verify
generated reads, invalid parameter errors, and restart refusal when the client
does not support elicitation, plus bounded health-check completion through both
flat tools and default Code Mode. Mutations are independently
verified through the services' HTTP APIs. Temporary profiles and tags are deleted
in `finally` blocks. An interrupted process can leave a `yarr-lab-*` fixture;
inspect those only in these disposable containers before removing them.

Plex checks cover generated identity and anonymous library access. With media
fixtures enabled, the runner also creates temporary movie/TV libraries, refreshes
them, independently verifies indexed content, and removes only those temporary
libraries. An HTTP 401/403 is reported as skipped and the report outcome becomes
incomplete. No account claim is required or attempted; account administration
and authenticated remote access are outside this lab's coverage.

## Media fixtures

With `ffmpeg` installed, add `--seed` to generate and scan one synthetic movie and
one synthetic TV episode:

```sh
python3 tests/media-lab/lab.py test --binary target/debug/yarr --seed
```

The files are ten-minute blue videos with silent audio. Metadata lookup uses Big
Buck Bunny (2008) and The Office (US), S01E01; these are **not copies of those
works**. Metadata services must be reachable. Both entries are unmonitored and
search-on-add is disabled. No download clients or indexers are configured. The
runner waits for each scan command to complete and verifies that the file API
contains this run's unique filename. Media, root folders, and library entries remain
for manual testing; subsequent runs replace files carrying the synthetic fixture
marker and reuse the same unmonitored entries. Plex can read the media volume.

For personally supplied media, download copies to an ignored local directory,
then create a private manifest (paths are local to the Linux/WSL runner):

```json
{
  "version": 1,
  "radarr": {
    "file": "/private/fixtures/movie.mkv", "metadataId": 10378,
    "size": 12345, "sha256": "REPLACE_WITH_SHA256"
  },
  "sonarr": {
    "file": "/private/fixtures/episode.mkv", "metadataId": 73244,
    "seasonNumber": 1, "episodeNumbers": [1],
    "size": 12345, "sha256": "REPLACE_WITH_SHA256"
  }
}
```

Use the actual TMDB movie ID or TVDB series ID and episode numbers for your files.
Replace the example byte sizes and SHA-256 values with `stat` and `sha256sum`
results. Keep the manifest and files under `.cache/real-media/`, which is ignored.

```sh
python3 tests/media-lab/lab.py test --binary target/debug/yarr \
  --media-manifest .cache/real-media/manifest.json
```

Both files are validated before API work. The runner copies them into its named
media volume, matches manual-import candidates, imports with copy mode, and
rescans. It checks the new command-outcome envelope against independent HTTP
reads and verifies imported sizes and SHA-256 hashes. Only filenames carrying
the runner's `YarrReal` marker are replaced on subsequent runs; unique staging
directories are cleaned in `finally` after completed imports. If command outcome
remains unknown, staging is retained so an active import cannot lose its source.
The harness resumes timed-out jobs by command ID for up to two minutes without
resubmission. Library entries and copies remain
unmonitored for inspection. Naming must preserve original filenames (the lab
defaults); changed naming rules cause a verification failure.

Reports omit private titles, paths, metadata IDs, byte counts, and hashes. Private error
details stay under `.cache/real-media/`. Do not commit the manifest, media, or
those diagnostics. Neither fixture runner connects to production storage.

## Addresses and lifecycle

| Service | Local address |
| --- | --- |
| Sonarr | http://127.0.0.1:18989 |
| Radarr | http://127.0.0.1:17878 |
| Plex | http://127.0.0.1:32401/web |

```sh
python3 tests/media-lab/lab.py down
```

Stopping preserves all named volumes. No command in the runner deletes volumes.
Private fixture copies therefore remain in the named media volume after stopping;
remove that specific lab volume explicitly when you no longer need the copies.
Containers restart with their Docker daemon unless explicitly stopped.

On the development Windows machine, Docker runs in the separate `codex-yarr-dev`
WSL distribution. Invoke these commands with `wsl -d codex-yarr-dev -- ...` and use
`--binary /root/yarr-target/debug/yarr`. Building with
`CARGO_TARGET_DIR=/root/yarr-target` keeps Rust artifacts on the Linux filesystem.
Keep a WSL shell open while using the lab: systemd services alone do not prevent
WSL from stopping an idle distribution. Windows localhost forwarding normally
makes the addresses above available from the host browser.

These are live smoke and CRUD checks, **not exhaustive endpoint coverage**. They
complement the Rust tests for validation and MCP approval behavior. The separate
legacy `xtask live` harness still targets its original backuphost environment.
