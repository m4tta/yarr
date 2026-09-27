# qBittorrent live lab

This harness validates Yarr's curated and generated qBittorrent controls against a disposable,
pinned qBittorrent 5.2.3 container. It creates one tracker-free private torrent
from local synthetic bytes, disables DHT/PeX/LSD, and independently verifies each
Yarr mutation through the qBittorrent API or the lab volume.

It exercises 17 convenience-control cases and 26 generated API operations,
including settings, torrent files, trackers, queue priority, force start, share
limits, automatic management, moving and renaming files, recheck, multipart
upload/metadata parsing, and binary export. It proves deletion both preserves
and removes the synthetic payload when requested. RSS, search plugins, peers,
and logs receive representative reads; every API operation is not live-tested.
The complete 121-operation contract is separately audited against upstream
source by `scripts/check-qbittorrent-openapi.py`.
The last verified live run is recorded in [results.json](results.json).

Run it inside the `codex-yarr-dev` WSL distro after building Yarr:

```sh
python3 tests/qbit-lab/lab.py test --binary /root/yarr-target/debug/yarr
python3 -m unittest discover -s tests/qbit-lab -p 'test_*.py'
```

`test` starts with fresh owned resources, verifies authentication is required,
and uses the container's inspected private bridge address. This avoids Windows/
WSL localhost forwarding conflicts. Every Docker invocation is pinned to the
local Unix socket, regardless of the selected Docker context. The published
debug port remains bound to loopback.

The test removes its named container, network, and volumes on success or
failure. `up`, `status`, and `down` are available for troubleshooting. The guard
rejects remote Docker endpoints, foreign resources, non-volume mounts, altered
images, and non-loopback host bindings.
