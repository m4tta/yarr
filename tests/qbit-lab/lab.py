#!/usr/bin/env python3
"""Rerunnable live verification against one disposable qBittorrent 5.2.3 lab."""

from __future__ import annotations

import argparse
import base64
import http.cookiejar
import ipaddress
import json
import os
import pathlib
import re
import subprocess
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid

from guard import CONTAINER, PORT, compose_action, inspect_lab, _docker as docker
from torrent import create_fixture
from operations import run_generated

HERE = pathlib.Path(__file__).resolve().parent
COMPOSE = HERE / "compose.yaml"
BASE_URL = f"http://127.0.0.1:{PORT}"

def temporary_password() -> str:
    logs = docker("logs", CONTAINER).stdout
    matches = re.findall(r"temporary password[^:\n]*:\s*(\S+)", logs, re.I)
    if not matches:
        raise RuntimeError("qBittorrent did not publish a temporary lab password")
    return matches[-1]


class QbitApi:
    def __init__(self, password: str):
        self.password = password
        cookies = http.cookiejar.CookieJar()
        self.opener = urllib.request.build_opener(
            urllib.request.ProxyHandler({}),
            urllib.request.HTTPCookieProcessor(cookies)
        )
        result = self.request(
            "POST", "/api/v2/auth/login", {"username": "admin", "password": password}
        )
        if result not in (None, "Ok."):
            raise RuntimeError("qBittorrent lab login failed")

    def request(self, method: str, path: str, fields: dict[str, object] | None = None):
        data = None
        headers = {"Referer": BASE_URL}
        if fields is not None:
            data = urllib.parse.urlencode(fields).encode()
            headers["Content-Type"] = "application/x-www-form-urlencoded"
        request = urllib.request.Request(BASE_URL + path, data=data, headers=headers, method=method)
        try:
            with self.opener.open(request, timeout=10) as response:
                body = response.read()
                content_type = response.headers.get_content_type()
        except urllib.error.HTTPError as error:
            raise RuntimeError(f"qBittorrent API returned HTTP {error.code}") from None
        if not body:
            return None
        text = body.decode("utf-8")
        if content_type == "application/json" or text[:1] in "[{":
            return json.loads(text)
        return text

    def get(self, path: str):
        return self.request("GET", path)

    def post(self, path: str, **fields: object):
        return self.request("POST", path, fields)

    def add_torrent(self, torrent: pathlib.Path) -> None:
        boundary = "----yarrqbitlab" + uuid.uuid4().hex
        chunks: list[bytes] = []

        def field(name: str, value: str) -> None:
            chunks.extend(
                [
                    f"--{boundary}\r\n".encode(),
                    f'Content-Disposition: form-data; name="{name}"\r\n\r\n'.encode(),
                    value.encode(),
                    b"\r\n",
                ]
            )

        field("savepath", "/downloads")
        field("stopped", "true")
        chunks.extend(
            [
                f"--{boundary}\r\n".encode(),
                (
                    'Content-Disposition: form-data; name="torrents"; '
                    f'filename="{torrent.name}"\r\n'
                ).encode(),
                b"Content-Type: application/x-bittorrent\r\n\r\n",
                torrent.read_bytes(),
                b"\r\n",
                f"--{boundary}--\r\n".encode(),
            ]
        )
        request = urllib.request.Request(
            BASE_URL + "/api/v2/torrents/add",
            data=b"".join(chunks),
            headers={
                "Content-Type": f"multipart/form-data; boundary={boundary}",
                "Referer": BASE_URL,
            },
            method="POST",
        )
        try:
            with self.opener.open(request, timeout=10) as response:
                if response.status != 200:
                    raise RuntimeError("qBittorrent rejected the synthetic torrent")
        except urllib.error.HTTPError as error:
            raise RuntimeError(f"qBittorrent add returned HTTP {error.code}") from None


class Yarr:
    def __init__(self, binary: pathlib.Path, password: str):
        if not binary.is_file():
            raise RuntimeError(f"yarr binary does not exist: {binary}")
        self.binary = binary
        self.password = password

    def action(self, name: str, params: dict[str, object] | None = None):
        params = params or {}
        code = f"async () => await qbit[{json.dumps(name)}]({json.dumps(params)})"
        with tempfile.TemporaryDirectory(prefix="yarr-qbit-home-") as home:
            env = {
                key: value
                for key, value in os.environ.items()
                if not key.startswith(("YARR_", "RUSTARR_"))
            }
            env.update(
                {
                    "HOME": home,
                    "YARR_SERVICES": "qbit",
                    "YARR_QBIT_KIND": "qbittorrent",
                    "YARR_QBIT_URL": BASE_URL,
                    "YARR_QBIT_USERNAME": "admin",
                    "YARR_QBIT_PASSWORD": self.password,
                    "NO_PROXY": urllib.parse.urlsplit(BASE_URL).hostname + ",127.0.0.1,localhost",
                }
            )
            result = subprocess.run(
                [str(self.binary), "codemode", "--code", code],
                capture_output=True,
                text=True,
                env=env,
                timeout=45,
            )
        if result.returncode:
            message = (result.stderr or result.stdout).replace(self.password, "<redacted>")
            raise RuntimeError(message.strip().splitlines()[-1])
        return json.loads(result.stdout)["result"]


def wait_for(label: str, predicate, timeout: float = 20.0):
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        last = predicate()
        if last:
            return last
        time.sleep(0.25)
    raise RuntimeError(f"timed out verifying {label}")


def torrent_info(api: QbitApi, info_hash: str) -> dict | None:
    query = urllib.parse.urlencode({"hashes": info_hash})
    rows = api.get("/api/v2/torrents/info?" + query)
    return rows[0] if rows else None


def require_submitted(value, action: str) -> None:
    if not isinstance(value, dict) or value.get("submitted") is not True:
        raise RuntimeError(f"{action} did not return a submitted envelope")


def copy_payload(payload: pathlib.Path) -> None:
    docker("cp", str(payload), f"{CONTAINER}:/downloads/{payload.name}")
    docker("exec", CONTAINER, "chown", f"1000:1000", f"/downloads/{payload.name}")


def file_exists(name: str) -> bool:
    return docker("exec", CONTAINER, "test", "-f", f"/downloads/{name}", check=False).returncode == 0


def run_live(binary: pathlib.Path) -> dict:
    global BASE_URL
    info = inspect_lab(require_running=True)
    address = ipaddress.ip_address(next(iter(info["NetworkSettings"]["Networks"].values()))["IPAddress"])
    if not isinstance(address, ipaddress.IPv4Address) or not address.is_private:
        raise RuntimeError("lab container must have a private IPv4 bridge address")
    BASE_URL = f"http://{address}:8080"
    try:
        urllib.request.build_opener(urllib.request.ProxyHandler({})).open(
            BASE_URL + "/api/v2/app/preferences", timeout=10
        ).close()
        raise RuntimeError("lab authentication must be enabled")
    except urllib.error.HTTPError as error:
        if error.code not in (401, 403):
            raise RuntimeError(f"unexpected unauthenticated response {error.code}") from None
    password = temporary_password()
    api = QbitApi(password)
    yarr = Yarr(binary, password)
    version = api.get("/api/v2/app/version")
    if version != "v5.2.3":
        raise RuntimeError(f"unexpected qBittorrent version {version!r}")

    api.post("/api/v2/app/setPreferences", json=json.dumps({"dht": False, "pex": False, "lsd": False}))
    preferences = api.get("/api/v2/app/preferences")
    if any(preferences.get(name) is not False for name in ("dht", "pex", "lsd")):
        raise RuntimeError("peer discovery was not disabled")

    nonce = uuid.uuid4().hex[:10]
    category = f"lab-{nonce}"
    tags = [f"lab-{nonce}-a", f"lab-{nonce}-b"]
    actions: list[str] = []

    with tempfile.TemporaryDirectory(prefix="yarr-qbit-fixture-") as root:
        payload, torrent, info_hash = create_fixture(pathlib.Path(root), nonce)
        copy_payload(payload)
        api.add_torrent(torrent)
        wait_for("synthetic torrent admission", lambda: torrent_info(api, info_hash))

        queue = yarr.action("download_queue")
        actions.append("download_queue")
        if not any(row.get("hash") == info_hash for row in queue):
            raise RuntimeError("curated queue omitted the synthetic torrent")

        require_submitted(yarr.action("download_resume", {"hash": info_hash}), "resume")
        actions.append("download_resume")
        wait_for(
            "torrent start",
            lambda: (row := torrent_info(api, info_hash))
            and not str(row.get("state", "")).startswith("stopped"),
        )
        wait_for(
            "local payload verification",
            lambda: (row := torrent_info(api, info_hash)) and row.get("progress") == 1,
            timeout=40,
        )

        require_submitted(yarr.action("download_pause", {"hash": info_hash}), "pause")
        actions.append("download_pause")
        wait_for(
            "torrent stop",
            lambda: (row := torrent_info(api, info_hash))
            and str(row.get("state", "")).startswith("stopped"),
        )
        require_submitted(yarr.action("download_resume", {"hash": info_hash}), "resume")
        wait_for(
            "torrent restart",
            lambda: (row := torrent_info(api, info_hash))
            and not str(row.get("state", "")).startswith("stopped"),
        )

        transfer = yarr.action("download_transfer")
        actions.append("download_transfer")
        direct_transfer = api.get("/api/v2/transfer/info")
        if transfer.get("connection_status") != direct_transfer.get("connection_status"):
            raise RuntimeError("curated transfer status disagreed with independent readback")

        require_submitted(
            yarr.action(
                "download_set_limits", {"download_limit": 65536, "upload_limit": 32768}
            ),
            "global limits",
        )
        actions.append("download_set_limits")
        wait_for(
            "global limits",
            lambda: (value := api.get("/api/v2/transfer/info"))
            and value.get("dl_rate_limit") == 65536
            and value.get("up_rate_limit") == 32768,
        )
        require_submitted(
            yarr.action(
                "download_set_limits",
                {"hash": info_hash, "download_limit": 16384, "upload_limit": 8192},
            ),
            "torrent limits",
        )
        wait_for(
            "torrent limits",
            lambda: (row := torrent_info(api, info_hash))
            and row.get("dl_limit") == 16384
            and row.get("up_limit") == 8192,
        )

        require_submitted(
            yarr.action("download_create_category", {"category": category}), "create category"
        )
        actions.append("download_create_category")
        wait_for("category creation", lambda: category in api.get("/api/v2/torrents/categories"))
        require_submitted(
            yarr.action(
                "download_edit_category", {"category": category, "save_path": "/downloads"}
            ),
            "edit category",
        )
        actions.append("download_edit_category")
        wait_for(
            "category edit",
            lambda: api.get("/api/v2/torrents/categories")
            .get(category, {})
            .get("savePath")
            == "/downloads",
        )
        categories = yarr.action("download_categories")
        actions.append("download_categories")
        if category not in categories:
            raise RuntimeError("curated categories omitted the created category")
        require_submitted(
            yarr.action("download_set_category", {"hash": info_hash, "category": category}),
            "set category",
        )
        actions.append("download_set_category")
        wait_for("category assignment", lambda: torrent_info(api, info_hash).get("category") == category)
        require_submitted(
            yarr.action("download_set_category", {"hash": info_hash}), "clear category"
        )
        wait_for("category clearing", lambda: torrent_info(api, info_hash).get("category") == "")
        require_submitted(
            yarr.action("download_remove_category", {"category": category}), "remove category"
        )
        actions.append("download_remove_category")
        wait_for("category removal", lambda: category not in api.get("/api/v2/torrents/categories"))

        require_submitted(yarr.action("download_create_tags", {"tags": tags}), "create tags")
        actions.append("download_create_tags")
        wait_for("tag creation", lambda: set(tags) <= set(api.get("/api/v2/torrents/tags")))
        listed_tags = yarr.action("download_tags")
        actions.append("download_tags")
        if not set(tags) <= set(listed_tags):
            raise RuntimeError("curated tags omitted created tags")
        require_submitted(
            yarr.action("download_add_tags", {"hash": info_hash, "tags": tags}), "add tags"
        )
        actions.append("download_add_tags")
        wait_for(
            "tag assignment",
            lambda: set(filter(None, torrent_info(api, info_hash).get("tags", "").split(", ")))
            == set(tags),
        )
        require_submitted(
            yarr.action("download_remove_tags", {"hash": info_hash, "tags": [tags[0]]}),
            "remove one tag",
        )
        actions.append("download_remove_tags")
        wait_for(
            "single tag removal",
            lambda: torrent_info(api, info_hash).get("tags") == tags[1],
        )
        require_submitted(
            yarr.action("download_remove_tags", {"hash": info_hash}), "remove all tags"
        )
        wait_for("all tag removal", lambda: torrent_info(api, info_hash).get("tags") == "")
        require_submitted(yarr.action("download_delete_tags", {"tags": tags}), "delete tags")
        actions.append("download_delete_tags")
        wait_for("tag deletion", lambda: not set(tags) & set(api.get("/api/v2/torrents/tags")))

        generated_actions = run_generated(api, yarr, info_hash, payload, torrent, wait_for)

        require_submitted(
            yarr.action("download_remove", {"hash": info_hash, "delete_files": False}),
            "remove keeping data",
        )
        actions.append("download_remove_keep_data")
        wait_for("torrent removal", lambda: torrent_info(api, info_hash) is None)
        if not file_exists(payload.name):
            raise RuntimeError("delete_files=false removed the payload")

        yarr.action("post_torrents_add", {
            "multipartFileBase64": base64.b64encode(torrent.read_bytes()).decode(),
            "fileName": torrent.name,
            "body": {"savepath": "/downloads", "stopped": True},
        })
        generated_actions.append("post_torrents_add")
        wait_for("generated multipart torrent re-admission", lambda: torrent_info(api, info_hash))
        require_submitted(
            yarr.action("download_remove", {"hash": info_hash, "delete_files": True}),
            "remove deleting data",
        )
        actions.append("download_remove_with_data")
        wait_for("torrent deletion", lambda: torrent_info(api, info_hash) is None)
        wait_for("payload deletion", lambda: not file_exists(payload.name))

    api.post("/api/v2/transfer/setDownloadLimit", limit=0)
    api.post("/api/v2/transfer/setUploadLimit", limit=0)
    return {
        "status": "passed",
        "qbittorrent_version": version.removeprefix("v"),
        "curated_check_count": len(actions),
        "curated_checks": actions,
        "generated_operation_count": len(generated_actions),
        "generated_operations": generated_actions,
        "synthetic_torrents": 1,
        "external_media_downloads": 0,
        "endpoint": "owned local Docker bridge",
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("command", choices=("up", "status", "test", "down"))
    parser.add_argument(
        "--binary", type=pathlib.Path, default=pathlib.Path("/root/yarr-target/debug/yarr")
    )
    args = parser.parse_args()
    if args.command == "up":
        compose_action("up", COMPOSE)
        print(json.dumps({"status": "ready", "endpoint": f"127.0.0.1:{PORT}"}))
    elif args.command == "status":
        info = inspect_lab()
        print(json.dumps({"running": bool(info and info.get("State", {}).get("Running"))}))
    elif args.command == "down":
        compose_action("down", COMPOSE)
        print(json.dumps({"status": "removed"}))
    else:
        compose_action("down", COMPOSE)
        compose_action("up", COMPOSE)
        try:
            report = run_live(args.binary)
        finally:
            compose_action("down", COMPOSE)
        report["cleanup"] = "complete"
        print(json.dumps(report, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
