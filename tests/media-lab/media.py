"""Synthetic media fixture: no downloads or production media access."""

from pathlib import Path
import re
import tempfile
import time
import uuid

from lab import require, run


def seed_and_scan(lab, service):
    """Leave one unmonitored fixture in each isolated service for manual testing."""
    movie = service == "radarr"
    root = "/data/movies" if movie else "/data/tv"
    folder = f"{root}/Big Buck Bunny (2008)" if movie else f"{root}/The Office (US)/Season 01"
    basename = "Big Buck Bunny (2008) - WEBDL-720p" if movie else "The Office (US) - S01E01 - WEBDL-720p"
    filename = f"{basename}-YarrLab{uuid.uuid4().hex[:12]}.mkv"
    container = lab.container(service)
    run("docker", "exec", container, "mkdir", "-p", folder)
    run("docker", "exec", container, "chown", "-R", "1000:1000", root)
    roots = lab.op(service, "get_rootfolder")
    if not any(item["path"] == root for item in roots):
        lab.op(service, "post_rootfolder", body={"path": root})
    existing = lab.op(service, "get_movie" if movie else "get_series")
    target_id, field = (10378, "tmdbId") if movie else (73244, "tvdbId")
    target = next((item for item in existing if item.get(field) == target_id), None)
    if target is None:
        target = (lab.op(service, "get_movie_lookup_tmdb", tmdbId=target_id) if movie
                  else lab.op(service, "get_series_lookup", term=f"tvdb:{target_id}")[0])
        target.update(rootFolderPath=root, qualityProfileId=lab.op(service, "get_qualityprofile")[0]["id"],
                      monitored=False, addOptions={"searchForMovie" if movie else "searchForMissingEpisodes": False})
        target.pop("id", None)
        if not movie:
            target.update(seasonFolder=True)
            for season in target.get("seasons", []):
                season["monitored"] = False
        target = lab.op(service, "post_movie" if movie else "post_series", body=target)
    require(target["monitored"] is False, "Fixture was enabled for monitoring; disable it in the lab before rerunning")
    actual_folder = target["path"] if movie else target["path"] + "/Season 01"
    require(actual_folder.startswith(root + "/") and ".." not in Path(actual_folder).parts,
            "Fixture path escaped the isolated media root")
    # Delete only previously indexed synthetic fixture files, so a stale record
    # cannot satisfy this run's unique-filename verification.
    files_path = f"/api/v3/{'moviefile?movieId' if movie else 'episodefile?seriesId'}={target['id']}"
    for previous in lab.http(service, files_path):
        if re.fullmatch(re.escape(basename) + r"-YarrLab[0-9a-f]{12}\.mkv", Path(previous.get("relativePath", "")).name):
            lab.op(service, "delete_moviefile_by_id" if movie else "delete_episodefile_by_id", id=previous["id"])
    run("docker", "exec", container, "mkdir", "-p", actual_folder)
    for path in run("docker", "exec", container, "find", actual_folder, "-maxdepth", "1", "-type", "f").splitlines():
        if Path(path).parent == Path(actual_folder) and re.fullmatch(re.escape(basename) + r"-YarrLab[0-9a-f]{12}\.mkv", Path(path).name):
            run("docker", "exec", container, "rm", "--", path)
    with tempfile.TemporaryDirectory(prefix="yarr-synthetic-") as temporary:
        clip = Path(temporary) / "synthetic.mkv"
        run("ffmpeg", "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i",
            "color=c=blue:s=1280x720:r=1", "-f", "lavfi", "-i", "anullsrc=r=48000:cl=stereo",
            "-t", "600", "-c:v", "libx264", "-preset", "ultrafast", "-crf", "35", "-c:a", "aac",
            "-b:a", "32k", "-metadata", "comment=Synthetic Yarr test fixture; not original media", str(clip))
        # Use the API-selected path to accommodate each service's naming defaults.
        run("docker", "exec", container, "mkdir", "-p", actual_folder)
        run("docker", "cp", str(clip), f"{container}:{actual_folder}/{filename}")
        run("docker", "exec", container, "chown", "-R", "1000:1000", root)
    command = lab.op(service, "post_command", body={"name": "RescanMovie" if movie else "RescanSeries",
                      "movieId" if movie else "seriesId": target["id"]})
    deadline = time.monotonic() + 90
    while time.monotonic() < deadline:
        state = lab.op(service, "get_command_by_id", id=command["id"])
        if state["status"] == "completed":
            break
        if state["status"] in {"failed", "aborted", "cancelled"}:
            raise AssertionError(f"Scan command {state['status']}")
        time.sleep(1)
    else:
        raise AssertionError("Scan did not finish within 90 seconds")
    files = lab.http(service, files_path)
    require(any(item.get("relativePath", "").endswith(filename) for item in files), "Completed scan did not index the current fixture")
    return {"fixture": "generated 10-minute blue video with silent audio (not original media)",
            "metadata": "Big Buck Bunny (2008)" if movie else "The Office (US), S01E01",
            "verified": "scan command completed and independent file API found current unique fixture",
            "monitoring": "entry unmonitored; search-on-add disabled for new entries"}
