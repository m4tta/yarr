---
title: "yarr API"
created: 2026-05-22
updated: 2026-09-27
---

# yarr API

`yarr` exposes **one MCP tool named `yarr`** and a service-grouped CLI. Both
surfaces call `YarrService`. The MCP tool runs Code Mode; the CLI maps verbs to
the same underlying actions.

## MCP Tool

Tool name: `yarr`

| Field | Type | Required | Notes |
|---|---|---:|---|
| `code` | string | yes | A JavaScript async arrow function executed in the Code Mode sandbox (the `codemode` action). It reaches the whole fleet via per-service callables and returns `{ result, calls, logs, artifacts }`. |

Inside `code` you have:

- **Per-service callables** with the service baked in — table-driven operations
  for the 7 spec-backed services and curated commands for download, stats,
  subtitles, and trace capabilities. Use `codemode.describe()` for the exact
  parameters available in this build.
- **Raw passthrough**: `api.<service>.get/post/put/delete(path, body)`.
- **Escape hatch**: `callTool(action, params)` dispatches any action directly.
- **Discovery**: `codemode.search(query)` lists matching callables;
  `codemode.describe(path)` returns a callable's signature or a response type's
  TypeScript interface.
- **Snippets / artifacts**: `codemode.run(name, input)`, `codemode.snippets()`,
  `writeArtifact(path, content, options?)`.

Example `yarr` call:

```json
{"name":"yarr","arguments":{"code":"async () => { const s = await sonarr.get_system_status(); return s.version; }"}}
```

```js
// A richer script: discover, then act across services.
async () => {
  const queue = await radarr.get_queue();
  await radarr.post_command({ body: { name: "MoviesSearch", movieIds: [456] } });
  return { queued: queue.records?.length };
}
```

### Action dispatch shape

Underneath, every callable / `callTool` / CLI verb resolves to one action. The
dispatch arguments are:

| Field | Type | Required | Notes |
|---|---|---:|---|
| `action` | string | yes | A generic action (`service_status`, `api_get`, `api_post`, `api_put`, `api_delete`, `help`, `codemode`, `op`, `snippet_list`, `snippet_save`, `snippet_run`, `snippet_delete`) or a curated command (`download_*`, `stats_*`) |
| `service` | string | action-dependent | Configured service name such as `sonarr` or `radarr` (baked into per-service callables, so scripts never pass it) |
| `path` | string | action-dependent | Relative upstream API path for the generic passthrough actions |
| `body` | object | no | JSON body forwarded upstream for `api_post`/`api_put`; defaults to `{}` |

There is no `confirm` parameter or MCP confirmation prompt. After transport
authentication, action scope checks, and input validation, calls execute
immediately on CLI and MCP, including `callTool` and operation calls nested in
Code Mode. Clients and agents must treat explicit user instructions as
authorization and clarify ambiguous requests before calling Yarr.

Generated operations are dispatched via the `op` action (`{action:"op", service, op, args}`); inside Code Mode they are the per-service callables above. The action set is **registry-derived** — run the `help` action (or `yarr help`) for the current full list and per-action params.

### Waiting for Sonarr and Radarr commands

`post_command` normally returns the upstream submission response immediately.
Opt into bounded completion tracking with `waitForCompletion` alongside `body`:

```js
async () => sonarr.post_command({
  body: { name: "RescanSeries", seriesId: 123 },
  waitForCompletion: { timeoutSeconds: 20, pollIntervalMs: 500 }
})
```

The equivalent CLI arguments are:

```sh
yarr sonarr op post_command --args '{"body":{"name":"RescanSeries","seriesId":123},"waitForCompletion":{}}'
```

Both fields are optional integers: `timeoutSeconds` defaults to 20 and accepts
1-20; `pollIntervalMs` defaults to 500 and accepts 100-5000. These controls apply
only to Sonarr/Radarr `post_command`; invalid controls fail before submission.
Yarr submits once, then polls within one deadline including HTTP request time.

The result contains `commandId`, `status`, `finished`, and the latest upstream
`command`. Status is `completed`, `failed`, `aborted`, `cancelled`, `timed_out`,
or `unknown`; upstream `orphaned` maps to `failed`. `finished` means an upstream
terminal state was observed, so callers must inspect `status` to decide success.
Timeouts and failed/malformed polls preserve the command ID and resumability
guidance. Use `get_command_by_id` to check again; a timeout does not cancel the
upstream job. If submission fails before an ID is returned, its outcome may be
unknown: do not blindly resubmit. These semantics are shared by CLI, flat MCP,
and Code Mode. Other operations retain their normal response shape.

### Generated-operation support boundary

The vendored specs generate runtime metadata tables, not a dedicated Rust
function for each operation. The executor preserves and enforces path, query,
header, and cookie parameters, including requiredness, schema, and OpenAPI
`style`/`explode` serialization (`simple`, `label`, `matrix`, `form`,
`spaceDelimited`, `pipeDelimited`, and `deepObject`). Supported request
representations include JSON (including `+json`), URL-encoded forms, multipart
text/files, text/XML, and raw binary bodies. Successful responses are negotiated
and decoded as JSON, text, or base64-encoded binary values.

Operations are omitted when their declared transport cannot be represented
losslessly. The generated, runtime-derived [supported/omitted capability
matrix](TOOLS_ACTIONS_ENDPOINTS.md#generated-operations-spec-backed-services)
is the public source of truth for current counts and exact omission reasons.
The omitted set currently consists of the Sonarr, Radarr, and Prowlarr
`get_by_path` operations whose declared path parameter has no path placeholder,
plus Overseerr's `get_settings_plex_library` operation whose `enable` query
parameter requires unsupported `allowReserved` serialization. Jellyfin and Plex
have no omitted operations.

## CLI Parity

The CLI is **service-grouped** (`yarr <service> <command> [flags]`); there is
no `--service` flag. Infra commands (`help`, `codemode`, `snippet …`) are
service-less.

```bash
yarr help
yarr radarr status
yarr sonarr get --path /api/v3/system/status
yarr radarr post --path /api/v3/command --body '{"name":"RefreshMovie"}'
yarr sonarr put --path /api/v3/series/editor --body '{"seriesIds":[1],"qualityProfileId":4}'
yarr radarr delete --path /api/v3/movie/12

# table-driven operations (the 7 spec-backed services)
yarr sonarr op get_series
yarr radarr op post_command --args '{"body":{"name":"MoviesSearch","movieIds":[456]}}'

# curated commands (download / stats only)
yarr qbittorrent queue
yarr tautulli activity

# Code Mode
yarr codemode --code 'async () => sonarr.get_system_status()'
```

`api_post`/`api_put`/`api_delete` all run immediately after authorization and
validation on both CLI and MCP; no `--confirm` flag exists. CLI ↔ MCP
registration parity is mechanically enforced by `tests/parity.rs`.

## qBittorrent controls

Give each qBittorrent instance its own configured name, URL, username, and
password. For example, `qbit_movies` and `qbit_tv` each declare
`YARR_<NAME>_KIND=qbittorrent`; their sessions and cookie jars remain separate
even when they use different ports on the same host. Use the exact configured
name in calls, because `qbittorrent` is ambiguous when there are two instances.

```js
async () => {
  const torrents = await qbit_movies.download_queue();
  const transfer = await qbit_movies.download_transfer();
  return { count: torrents.length, transfer };
}
```

Queue entries include download/upload speed, ETA, progress, state, limits,
category, and tags. Return a summary or selected entries rather than the entire
queue when a client has many torrents.

| Operation | Code Mode call |
| --- | --- |
| Stop or start one torrent | `download_pause({hash})`, `download_resume({hash})` |
| Stop or start all torrents | `download_pause()`, `download_resume()` |
| Set global speed limits | `download_set_limits({download_limit, upload_limit})` |
| Set one torrent's limits | `download_set_limits({hash, download_limit, upload_limit})` |
| Read categories | `download_categories()` |
| Create a category | `download_create_category({category, save_path})` |
| Edit a category's save path | `download_edit_category({category, save_path})` |
| Remove a category | `download_remove_category({category})` |
| Assign or clear a category | `download_set_category({hash, category})`; omit `category` to clear |
| Read tags | `download_tags()` |
| Create or delete tags | `download_create_tags({tags:["review"]})`, `download_delete_tags({tags:["review"]})` |
| Add or remove torrent tags | `download_add_tags({hash, tags:["review"]})`, `download_remove_tags({hash, tags:["review"]})` |
| Clear all tags on one torrent | `download_remove_tags({hash})` |
| Remove torrent, preserve data | `download_remove({hash, delete_files:false})` |
| Remove torrent and its data | `download_remove({hash, delete_files:true})` |

Prefix each call with its configured service, such as `qbit_tv.download_tags()`.
`id` is an alternative to `hash`; supply one selector, not both, for the new
controls. Limits are integer **bytes per second**, with `0` meaning unlimited.
Either limit can be omitted to preserve it. For example, 5 MiB/s is `5242880`.
qBittorrent stores global limits in KiB/s and may round a byte value; use a
multiple of 1024 and read back transfer status to verify the applied value.
Both requested limits are validated before any update, but two limits require
two upstream requests and are not an atomic transaction.

Category creation can omit `save_path` to use the client's default. Tags are
arrays of individual names; category/tag operations never delete torrent data.
Changing a category or its save path can affect where qBittorrent stores files.
The additional transfer, limit, category, and tag commands are qBittorrent-only;
the shared queue/add/pause/resume/remove commands also support SABnzbd.

Torrent deletion remains marked destructive even when preserving data, and
`delete_files` defaults to false. qBittorrent often returns
an empty successful response for writes, including when a hash does not exist;
read back the queue or corresponding setting to verify the intended change.

### Full WebUI API access

Beyond these convenience commands, qBittorrent has generated per-instance
operations from a locally maintained OpenAPI contract. They cover application
preferences, connection settings, logs, incremental synchronization, torrent
details and files, peers and trackers, queue and file priorities, recheck and
reannounce, share limits, storage and renaming, automatic management, RSS feeds
and rules, search and plugins, torrent-file upload, and binary export.

Use discovery for the exact operation and its parameter schema:

```js
async () => {
  return codemode.search("qbit_movies trackers");
}
```

Pass the returned callable path to `codemode.describe(path)`, then call that
method on `qbit_movies` or `qbit_tv`. GET parameters are top-level arguments;
POST form fields go in `body`. The executor applies form encoding automatically.
Upstream field names and separators are preserved: for example, `hashes` is a
pipe-separated string or the explicit string `all`, rather than a JSON array.
Use `download_*` helpers when you want single-torrent selectors and tag arrays.

For multipart uploads, supply `multipartFileBase64`, `fileName`, and any add
options in `body`; `contentType: "multipart/form-data"` selects that encoding.
Binary exports return a base64 response envelope. Limits on upload size,
response size, and Code Mode execution still apply.

The contract is audited against pinned qBittorrent 5.2.3 controller routes in
[`specs/qbittorrent-coverage.json`](../specs/qbittorrent-coverage.json). Authentication
is managed by Yarr's transport. Browser-only presentation controls are outside
the service API. Route coverage is distinct from live validation: the isolated
[qBittorrent lab](../tests/qbit-lab/README.md) tests representative operations
against a real server; it does not exercise every preference or search plugin.

MCP catalog metadata marks high-impact operations, including torrent deletion,
shutdown, API-key changes, application preferences, and plugin installation/update.
These labels are informative and do not gate execution or exhaustively describe
request-dependent effects, such as automatic-removal share limits.

## Security Rules

- `help` has no action scope, but mounted HTTP transports still require bearer/OAuth transport auth.
- `service_status` requires `yarr:read`.
- `api_get`, `api_post`, `api_put`, `api_delete`, `op`, and `codemode` require `yarr:write` because generic/credentialed upstream calls and arbitrary scripts can mutate services.
- `yarr:write` satisfies read.
- Paths with traversal or embedded query-string secrets are rejected.
- Responses are capped by the shared token-limit layer (and Code Mode shapes its envelope below that cap) before being returned to MCP clients.
