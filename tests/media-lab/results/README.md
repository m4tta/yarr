# Verification record — 2026-09-26

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
