"""Bounded Plex scans over the disposable lab's read-only media volume."""

from pathlib import PurePosixPath
import re
import subprocess
import time
import urllib.error
import uuid


_SECTIONS = (
    {
        "kind": "movie",
        "type": "movie",
        "scanner": "Plex Movie",
        "agent": "tv.plex.agents.movie",
        "location": "/data/movies",
        "expected_item_type": "movie",
    },
    {
        "kind": "show",
        "type": "show",
        "scanner": "Plex TV Series",
        "agent": "tv.plex.agents.series",
        "location": "/data/tv",
        "expected_item_type": "episode",
        "media_query": {"type": 4},
    },
)

_MARKER = re.compile(r"-Yarr(?:Lab|Real)[0-9a-f]{12}\.[^/]+$")


def _require(condition, message):
    if not condition:
        raise AssertionError(message)


def _directories(payload):
    container = payload.get("MediaContainer") if isinstance(payload, dict) else None
    if not isinstance(container, dict):
        raise AssertionError("Plex response has no MediaContainer")
    directories = container.get("Directory", [])
    if not isinstance(directories, list):
        raise AssertionError("Plex section response has no Directory list")
    return directories


def _metadata(payload):
    container = payload.get("MediaContainer") if isinstance(payload, dict) else None
    if not isinstance(container, dict):
        raise AssertionError("Plex response has no MediaContainer")
    metadata = container.get("Metadata", [])
    if not isinstance(metadata, list):
        raise AssertionError("Plex content response has no Metadata list")
    return metadata


def _section_id(section):
    value = section.get("key") if isinstance(section, dict) else None
    try:
        identifier = int(value)
    except (TypeError, ValueError) as error:
        raise AssertionError("Plex section has no numeric key") from error
    _require(identifier > 0, "Plex section key must be positive")
    return identifier


class PlexAdminUnavailable(RuntimeError):
    """The local Plex server requires credentials the lab intentionally lacks."""


def _admin_error(error):
    text = str(error)
    if "HTTP 401" in text or "HTTP 403" in text:
        raise PlexAdminUnavailable(
            "Plex library administration requires an account or token on this server; "
            "the media lab did not authenticate or claim it"
        ) from error
    raise error


def _http(lab, path):
    try:
        return lab.http("plex", path)
    except urllib.error.HTTPError as error:
        if error.code in {401, 403}:
            raise PlexAdminUnavailable(
                "Plex library reads require an account or token on this server; "
                "the media lab did not authenticate or claim it"
            ) from error
        raise


def _expected_files(lab):
    try:
        result = subprocess.run(
            [
                "docker",
                "exec",
                lab.container("plex"),
                "find",
                "/data/movies",
                "/data/tv",
                "-type",
                "f",
            ],
            check=True,
            capture_output=True,
            text=True,
            timeout=30,
        )
    except (OSError, subprocess.SubprocessError) as error:
        raise RuntimeError(
            "Could not inventory marked files in the isolated Plex container"
        ) from error

    expected = {"movie": set(), "show": set()}
    for path in result.stdout.splitlines():
        kind = (
            "movie"
            if path.startswith("/data/movies/")
            else "show"
            if path.startswith("/data/tv/")
            else None
        )
        if kind is not None and _MARKER.search(PurePosixPath(path).name):
            expected[kind].add(path)
    for kind, paths in expected.items():
        _require(
            paths,
            f"No marked {kind} fixtures are present in the isolated Plex volume",
        )
    return expected


def _part_files(items):
    files = set()
    for item in items:
        media_entries = item.get("Media", []) if isinstance(item, dict) else []
        if isinstance(media_entries, dict):
            media_entries = [media_entries]
        for media in media_entries:
            parts = media.get("Part", []) if isinstance(media, dict) else []
            if isinstance(parts, dict):
                parts = [parts]
            files.update(
                part["file"]
                for part in parts
                if isinstance(part, dict) and isinstance(part.get("file"), str)
            )
    return files


def _wait_for_section(lab, name, timeout=20):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        sections = _directories(_http(lab, "/library/sections"))
        match = next((section for section in sections if section.get("title") == name), None)
        if match is not None:
            return match
        time.sleep(0.5)
    raise AssertionError("Created Plex section did not appear within 20 seconds")


def _wait_for_content(
    lab,
    section_id,
    expected_type,
    expected_files,
    media_query=None,
    timeout=120,
):
    deadline = time.monotonic() + timeout
    suffix = "?type=4" if media_query == {"type": 4} else ""
    while time.monotonic() < deadline:
        latest = _metadata(_http(lab, f"/library/sections/{section_id}/all{suffix}"))
        types = {item.get("type") for item in latest if isinstance(item, dict)}
        indexed_files = _part_files(latest)
        if expected_type in types and expected_files <= indexed_files:
            return latest, indexed_files
        time.sleep(2)
    raise AssertionError(
        f"Plex {expected_type} section did not index all {len(expected_files)} marked files "
        "within 120 seconds"
    )


def check_plex_media(lab):
    """Create, scan, verify, and remove private lab-only Plex sections."""
    expected_files = _expected_files(lab)
    run_id = uuid.uuid4().hex[:12]
    client_id = f"yarr-media-lab-{run_id}"
    names = {
        section["kind"]: f"Yarr Lab {section['kind'].title()} {run_id}"
        for section in _SECTIONS
    }
    headers = {
        "X-Plex-Client-Identifier": client_id,
        "X-Plex-Product": "yarr-media-lab",
        "X-Plex-Version": "1",
        "accepts": "application/json",
    }
    created_ids = {}

    try:
        for section in _SECTIONS:
            try:
                lab.op(
                    "plex",
                    "add_section",
                    **headers,
                    name=names[section["kind"]],
                    type=section["type"],
                    scanner=section["scanner"],
                    agent=section["agent"],
                    language="en-US",
                    location=section["location"],
                )
            except RuntimeError as error:
                _admin_error(error)

            created = _wait_for_section(lab, names[section["kind"]])
            identifier = _section_id(created)
            created_ids[section["kind"]] = identifier
            try:
                lab.op(
                    "plex",
                    "refresh_section",
                    **headers,
                    sectionId=identifier,
                    force=1,
                )
            except RuntimeError as error:
                _admin_error(error)

        result = {}
        for section in _SECTIONS:
            identifier = created_ids[section["kind"]]
            direct_items, direct_files = _wait_for_content(
                lab,
                identifier,
                section["expected_item_type"],
                expected_files[section["kind"]],
                section.get("media_query"),
            )
            generated_args = {**headers, "sectionId": str(identifier)}
            if section.get("media_query"):
                generated_args["mediaQuery"] = section["media_query"]
            generated = lab.op(
                "plex",
                "list_content",
                **generated_args,
            )
            generated_items = _metadata(generated)
            generated_files = _part_files(generated_items)
            direct_types = sorted(
                {item.get("type") for item in direct_items if item.get("type")}
            )
            generated_types = sorted(
                {item.get("type") for item in generated_items if item.get("type")}
            )
            _require(
                generated_files == direct_files,
                "Generated and independent Plex indexed-file sets differ",
            )
            _require(
                generated_types == direct_types,
                "Generated and independent Plex content types differ",
            )
            result[section["kind"]] = {
                "item_count": len(direct_items),
                "expected_file_count": len(expected_files[section["kind"]]),
                "indexed_file_count": len(
                    direct_files & expected_files[section["kind"]]
                ),
                "types": direct_types,
            }

        return {
            "sections": result,
            "operations": [
                "add_section",
                "refresh_section",
                "list_content",
                "delete_library_section",
            ],
            "verified": (
                "generated create/refresh/read plus independent bounded verification "
                "of every marked Part.file path"
            ),
        }
    finally:
        sections = _directories(_http(lab, "/library/sections"))
        cleanup = {
            _section_id(section)
            for section in sections
            if section.get("title") in set(names.values())
        }
        cleanup.update(created_ids.values())
        for identifier in cleanup:
            try:
                lab.op(
                    "plex",
                    "delete_library_section",
                    **headers,
                    sectionId=str(identifier),
                    **{"async": 0},
                )
            except RuntimeError as error:
                _admin_error(error)
        remaining = _directories(_http(lab, "/library/sections"))
        _require(
            all(section.get("title") not in set(names.values()) for section in remaining),
            "Plex lab section cleanup did not persist",
        )
