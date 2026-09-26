# Verification record — 2026-09-26

## Real-media and command-outcome increment

[The second live report](2026-09-26-real-media.json) verifies the implementation
committed as `87771d6`. The run started before that commit, so it truthfully records
the preceding revision and `source_dirty: true`; the implementation was committed
unchanged after the run. The report includes the tested binary's SHA-256.

```sh
python3 tests/media-lab/lab.py test --binary /root/yarr-target/debug/yarr --seed \
  --media-manifest .cache/real-media/manifest.json \
  --report .cache/media-lab/final-candidate.json
```

| Check | Result |
| --- | --- |
| Full Rust workspace tests | 787 passed; 4 documentation examples ignored |
| Root package all-target clippy, warnings denied | Passed |
| Workspace formatting | Passed |
| Python lab unit tests | 22 passed |
| Live isolated lab | 21 passed; none skipped |
| Generated tool docs, schema docs, documentation links | Passed |
| Static pattern contracts | Passed with existing advisory file-size warnings |
| ASCII and diff checks | Passed |

The lab imported a private movie copy and episode copy, independently verified
completed import/rescan commands and file records, and matched imported SHA-256
hashes against the local sources. Both flat MCP and default Code Mode completed
health-check commands and agreed with independent upstream reads. Invalid and
unsupported wait options, submission/poll deadlines, failed commands, malformed
poll responses, and no-resubmission behavior are covered by Rust tests.

Custom formats and quality profiles completed CRUD checks; naming flags were
changed and restored exactly. Plex's generated library-creation route and query
contract were corrected after a live 404. Fresh Plex movie/TV sections indexed
all four exact fixture paths (two synthetic, two private copies); temporary Plex
sections were then removed. Private titles, paths, metadata IDs, hashes, and
media are excluded from the committed report.

Production access was limited to read-only metadata retrieval and copying two
source files over SSH. Lab media copies and unmonitored library entries remain
in isolated volumes for inspection. This verifies the listed workflows, not every
API endpoint, authenticated Plex account administration, download clients, or
indexer integration. Those areas remain follow-up work.

## Initial synthetic-media baseline

[The machine-readable live report](2026-09-26.json) records the implementation
revision, binary hash, image digests, service versions, and each check's outcome.
It was produced by:

```sh
python3 tests/media-lab/lab.py test --binary /root/yarr-target/debug/yarr --seed \
  --report tests/media-lab/results/2026-09-26.json
```

| Check | Result |
| --- | --- |
| `cargo test --workspace --no-fail-fast` | 775 passed; 4 documentation examples ignored |
| `cargo clippy --all-targets -- -D warnings` | Passed for the root yarr package |
| `cargo fmt -- --check` | Passed |
| `python3 -m unittest discover -s tests/media-lab -p 'test_*.py'` | 10 passed |
| Live lab | 14 passed, including two synthetic media scans and two stdio MCP sessions |
| `python3 scripts/check-doc-links.py` | Passed |
| `git diff --check` | Passed |

Rust tests ran on a Linux checkout because the Windows clone represents tracked
symlinks as text files; the repository's symlink invariant test correctly rejects
that Windows representation. Source changes were copied into the Linux checkout
and the tracked symlinks preserved before the full successful test run.

An additional `cargo clippy --workspace --all-targets -- -D warnings` run found
existing `clippy::doc_markdown` violations in unchanged `xtask` documentation
comments. The root package's required all-target lint gate passes. These unrelated
workspace-wide lint findings remain unresolved.

This establishes a smoke/CRUD baseline. It does not establish exhaustive API
coverage, authenticated Plex administration, real download processing, or end-user
workflow coverage. Media in this run is synthetic; no Unraid files were accessed.
