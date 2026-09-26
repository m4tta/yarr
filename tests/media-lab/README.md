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

Sonarr and Radarr checks cover status, quality profile create/read/update/delete,
tag create/delete, and invalid parameter rejection. Live stdio MCP sessions verify
generated reads, invalid parameter errors, and restart refusal when the client
does not support elicitation. Mutations are independently
verified through the services' HTTP APIs. Temporary profiles and tags are deleted
in `finally` blocks. An interrupted process can leave a `yarr-lab-*` fixture;
inspect those only in these disposable containers before removing them.

Plex checks cover generated identity and anonymous library access. An HTTP 401/403
on library access is reported as skipped and the report outcome becomes incomplete. No account claim is required
or attempted, and this is not evidence of authenticated Plex administration.

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

To use personally supplied media later, copy files into the lab volume rather
than mounting production storage. The synthetic run does not access Unraid.

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
