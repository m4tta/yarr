"""Import private, locally supplied copies into the guarded disposable lab."""

import hashlib
import json
import os
from pathlib import Path
import re
import traceback
import time
import uuid

from lab import ROOT, require, run


def load_manifest(path):
    """Validate both fixtures before making any API calls or copying into Docker."""
    manifest = json.loads(Path(path).read_text(encoding="utf-8-sig"))
    require(manifest.get("version") == 1, "Expected media manifest version 1")
    for service in ("sonarr", "radarr"):
        fixture = manifest[service]
        file = Path(fixture["file"]).expanduser().resolve(strict=True)
        require(file.is_file(), "Fixture must be a regular local file")
        require(file.suffix.lower() in {".mkv", ".mp4", ".avi", ".m4v"}, "Unsupported fixture extension")
        require(type(fixture["metadataId"]) is int and fixture["metadataId"] > 0, "Invalid metadata ID")
        require(type(fixture["size"]) is int and fixture["size"] > 0, "Invalid fixture size")
        require(file.stat().st_size == fixture["size"], "Fixture size mismatch")
        with file.open("rb") as stream:
            actual = hashlib.file_digest(stream, "sha256").hexdigest()
        require(actual == fixture["sha256"], "Fixture SHA-256 mismatch")
        fixture["file"] = str(file)
        if service == "sonarr":
            require(type(fixture["seasonNumber"]) is int and fixture["seasonNumber"] >= 0, "Invalid season")
            episodes = fixture["episodeNumbers"]
            require(isinstance(episodes, list) and episodes and
                    all(type(number) is int and number > 0 for number in episodes) and
                    len(set(episodes)) == len(episodes), "Invalid episode numbers")
    return manifest


def completed_command(lab, service, body):
    deadline = time.monotonic() + 120
    result = lab.op(service, "post_command", body=body,
                    waitForCompletion={"timeoutSeconds": 20, "pollIntervalMs": 250})
    if result.get("status") in {"timed_out", "unknown"}:
        # A bounded wait never cancels the upstream work. Resume by ID without
        # submitting again, and retain staged bytes if the outcome stays unknown.
        while time.monotonic() < deadline:
            state = lab.http(service, f"/api/v3/command/{result['commandId']}",
                             timeout=min(10, max(0.1, deadline - time.monotonic())))
            require(state.get("id") == result["commandId"], "Resumed command ID differs")
            if state.get("status") in {"completed", "failed", "aborted", "cancelled", "orphaned"}:
                result.update(status=state["status"], finished=True)
                break
            time.sleep(min(1, max(0, deadline - time.monotonic())))
    require(result.get("status") == "completed" and result.get("finished") is True,
            f"Command did not complete: {result.get('status')} (id {result.get('commandId')})")
    # Check the upstream command independently, not just yarr's returned envelope.
    require(lab.http(service, f"/api/v3/command/{result['commandId']}")["status"] == "completed",
            "Command completion disagrees with upstream")
    return result["commandId"]


def import_and_scan(lab, service, fixture):
    """Keep fixture metadata private even if an upstream error echoes its path."""
    diagnostic = ROOT / ".cache/real-media" / f"{service}-error.txt"
    try:
        result = _import_and_scan(lab, service, fixture)
        diagnostic.unlink(missing_ok=True)
        return result
    except Exception as error:
        diagnostic.parent.mkdir(parents=True, exist_ok=True)
        with os.fdopen(os.open(diagnostic, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600), "w", encoding="utf-8") as stream:
            os.chmod(diagnostic, 0o600)
            stream.write(lab.redact(traceback.format_exc()))
        raise RuntimeError(f"Real-media workflow failed ({type(error).__name__}); private diagnostic in .cache/real-media") from None


def _import_and_scan(lab, service, fixture):
    movie = service == "radarr"
    root = "/data/movies" if movie else "/data/tv"
    kind, field = ("movie", "tmdbId") if movie else ("series", "tvdbId")
    naming = lab.op(service, "get_config_naming")
    require(naming["renameMovies" if movie else "renameEpisodes"] is False,
            "Real-media lab requires original filenames; disable renaming before running")
    container = lab.container(service)
    run("docker", "exec", container, "mkdir", "-p", root)
    run("docker", "exec", container, "chown", "1000:1000", root)
    if not any(item["path"] == root for item in lab.op(service, "get_rootfolder")):
        lab.op(service, "post_rootfolder", body={"path": root})
    target = next((item for item in lab.op(service, f"get_{kind}")
                   if item.get(field) == fixture["metadataId"]), None)
    if target is None:
        target = (lab.op(service, "get_movie_lookup_tmdb", tmdbId=fixture["metadataId"]) if movie
                  else lab.op(service, "get_series_lookup", term=f"tvdb:{fixture['metadataId']}")[0])
        target.pop("id", None)
        target.update(rootFolderPath=root, monitored=False,
                      qualityProfileId=lab.op(service, "get_qualityprofile")[0]["id"],
                      addOptions={"searchForMovie" if movie else "searchForMissingEpisodes": False})
        if not movie:
            target["seasonFolder"] = True
            for season in target.get("seasons", []):
                season["monitored"] = False
        target = lab.op(service, f"post_{kind}", body=target)
    require(target["monitored"] is False, "Real-media lab entry must be unmonitored")
    require(target["path"].startswith(root + "/") and ".." not in Path(target["path"]).parts,
            "Library path escaped the lab root")
    run("docker", "exec", container, "mkdir", "-p", target["path"])
    run("docker", "exec", container, "chown", "1000:1000", target["path"])
    files_path = f"/api/v3/{'moviefile?movieId' if movie else 'episodefile?seriesId'}={target['id']}"
    marker = re.compile(r"-YarrReal[0-9a-f]{12}\.(mkv|mp4|avi|m4v)$")
    for previous in lab.http(service, files_path):
        if marker.search(Path(previous.get("relativePath", "")).name):
            lab.op(service, "delete_moviefile_by_id" if movie else "delete_episodefile_by_id", id=previous["id"])
    title = re.sub(r'[\\/:*?"<>|]', " ", target["title"])
    suffix = (f" ({target['year']})" if movie else
              f" - S{fixture['seasonNumber']:02}" + "".join(f"E{number:02}" for number in fixture["episodeNumbers"]))
    token = uuid.uuid4().hex[:12]
    filename = f"{title}{suffix}-YarrReal{token}{Path(fixture['file']).suffix.lower()}"
    staging = f"/data/downloads/yarr-real-{token}"
    source = f"{staging}/{filename}"
    run("docker", "exec", container, "mkdir", "-p", staging)
    cleanup_staging = True
    try:
        run("docker", "cp", fixture["file"], f"{container}:{source}")
        run("docker", "exec", container, "chown", "-R", "1000:1000", staging)
        # A seriesId/movieId selects files already in that library entry and
        # takes precedence over folder; folder alone inspects the staged copy.
        candidates = lab.op(service, "get_manualimport", folder=staging, filterExistingFiles=False)
        candidate = next((item for item in candidates if item["path"] == source), None)
        require(candidate is not None, f"Manual import did not return the staged file: {candidates!r}")
        require(candidate.get(kind, {}).get("id") == target["id"], "Manual import matched wrong library entry")
        item = {"path": source, f"{kind}Id": target["id"], "quality": candidate["quality"],
                "languages": candidate.get("languages", []), "releaseGroup": candidate.get("releaseGroup", "")}
        if not movie:
            expected = set(fixture["episodeNumbers"])
            episodes = candidate.get("episodes", [])
            require({ep["episodeNumber"] for ep in episodes} == expected and
                    all(ep["seasonNumber"] == fixture["seasonNumber"] for ep in episodes),
                    "Manual import matched wrong episode numbers")
            item["episodeIds"] = [ep["id"] for ep in episodes]
        cleanup_staging = False
        imported = completed_command(lab, service, {"name": "ManualImport", "files": [item], "importMode": "Copy"})
        cleanup_staging = True
        files = lab.http(service, files_path)
        current = next((item for item in files if Path(item.get("relativePath", "")).name == filename), None)
        require(current is not None and current["size"] == fixture["size"], "Manual import did not index this exact fixture")
        scanned = completed_command(lab, service, {"name": "RescanMovie" if movie else "RescanSeries",
                                                   f"{kind}Id": target["id"]})
        files = lab.http(service, files_path)
        require(any(item["id"] == current["id"] and item["size"] == fixture["size"] for item in files),
                "Rescan lost the imported fixture")
        indexed_path = target["path"] + "/" + current["relativePath"]
        require(".." not in Path(current["relativePath"]).parts and not Path(current["relativePath"]).is_absolute(),
                "Indexed path escaped the library")
        digest = run("docker", "exec", container, "sha256sum", "--", indexed_path).split()[0]
        require(digest == fixture["sha256"], "Imported bytes differ from the supplied copy")
        return {"fixture": "privately supplied local movie" if movie else "privately supplied local episode",
                "command_ids": {"import": imported, "scan": scanned},
                "verified": ["manual import matched metadata", "import and scan completion independently confirmed",
                             "file API size matched", "imported SHA-256 matched local source"],
                "monitoring": "unmonitored; search-on-add disabled"}
    finally:
        # Only the uniquely allocated staging directory is removed; library copies remain.
        if cleanup_staging:
            run("docker", "exec", container, "rm", "-rf", "--", staging)
