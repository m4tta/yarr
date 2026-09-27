# Vendored OpenAPI specs

Authoritative upstream API specs, vendored as the **source of truth** for the
Code Mode typed operation client codegen (see `xtask` generator + `src/codemode`).
Re-fetch with the URLs below when bumping.

| File | Service | Source |
|------|---------|--------|
| `sonarr.openapi.json`   | Sonarr (v3)   | github.com/Sonarr/Sonarr `src/Sonarr.Api.V3/openapi.json` (develop) |
| `radarr.openapi.json`   | Radarr (v3)   | github.com/Radarr/Radarr `src/Radarr.Api.V3/openapi.json` (develop) |
| `prowlarr.openapi.json` | Prowlarr (v1) | github.com/Prowlarr/Prowlarr `src/Prowlarr.Api.V1/openapi.json` (develop) |
| `overseerr.openapi.yml` | Overseerr     | github.com/sct/overseerr `overseerr-api.yml` (develop) |
| `jellyfin.openapi.json` | Jellyfin      | api.jellyfin.org `jellyfin-openapi-stable.json` |
| `plex.openapi.yml`      | Plex          | github.com/LukeHagar/plex-api-spec `plex-api-spec.yaml` (main) |
| `qbittorrent.openapi.json` | qBittorrent 5.2.3 | Locally maintained contract derived from the official WebUI API documentation and pinned upstream controller source; see `qbittorrent-coverage.json`. |

The remaining 4 services (Tautulli, SABnzbd, Bazarr, Tracearr) have no
machine-readable spec; their operations are derived from the endpoint annotations in
`src/models/<svc>.rs` doc comments / published docs.

qBittorrent does not publish this OpenAPI document upstream. Its local contract
uses the same generator and request executor as the vendored specs, with form
encoding, multipart torrent uploads, and binary exports. The source coverage
inventory distinguishes callable operations from transport-managed authentication
and includes the upstream revision used for the audit. Curated qBittorrent
download helpers remain available alongside generated operations.
