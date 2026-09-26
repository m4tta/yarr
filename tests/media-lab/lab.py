#!/usr/bin/env python3
"""Live checks against this directory's disposable Compose project only."""

import argparse
import copy
import datetime
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import urllib.error
import urllib.request
import uuid
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[2]
COMPOSE = Path(__file__).with_name("compose.yaml")
from guard import PORTS, compose_action, inspect_lab


def run(*args, **kwargs):
    return subprocess.run(args, check=True, capture_output=True, text=True, timeout=180, **kwargs).stdout


def require(condition, message):
    if not condition:
        raise AssertionError(message)


class Lab:
    def __init__(self, binary, home, containers):
        self.binary = str(Path(binary).resolve())
        self.containers = containers
        self.keys = {}
        self.env = {key: value for key, value in os.environ.items()
                    if not key.startswith(("YARR_", "RUSTARR_")) and key.upper() not in
                    {"HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"}}
        self.env.update(HOME=home, YARR_HOME=home, YARR_CONFIG=str(Path(home) / "config.toml"),
                        YARR_SERVICES="sonarr,radarr,plex", NO_PROXY="127.0.0.1,localhost")
        Path(home, "config.toml").write_text("", encoding="utf-8")
        for service, (port, _) in PORTS.items():
            prefix = f"YARR_{service.upper()}"
            self.env[f"{prefix}_URL"] = f"http://127.0.0.1:{port}"
            if service != "plex":
                xml = run("docker", "exec", self.container(service), "cat", "/config/config.xml")
                key = ET.fromstring(xml).findtext("ApiKey")
                if not key:
                    raise RuntimeError(f"{service} API key missing")
                self.keys[service] = key
                self.env[f"{prefix}_API_KEY"] = key

    def container(self, service):
        return self.containers[service]["Id"]

    def redact(self, text):
        for secret in self.keys.values():
            text = text.replace(secret, "[REDACTED]")
        return text

    def cli(self, service, *args, fail=False):
        result = subprocess.run([self.binary, service, *args], env=self.env, capture_output=True,
                                text=True, timeout=90)
        if fail:
            if result.returncode == 0:
                raise AssertionError("Invalid request unexpectedly succeeded")
            return self.redact(result.stderr)
        if result.returncode:
            raise RuntimeError(self.redact(result.stderr[-3000:]))
        return json.loads(result.stdout)

    def op(self, service, name, **args):
        return self.cli(service, "op", name, "--args", json.dumps(args))

    def http(self, service, path):
        headers = {"Accept": "application/json"}
        if service in self.keys:
            headers["X-Api-Key"] = self.keys[service]
        request = urllib.request.Request(self.env[f"YARR_{service.upper()}_URL"] + path, headers=headers)
        # Explicitly bypass user proxy settings for independent verification too.
        with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(request, timeout=30) as response:
            return json.load(response)

    def profile_roundtrip(self, service):
        profile = copy.deepcopy(self.op(service, "get_qualityprofile")[0])
        profile.pop("id", None)
        profile["name"] = f"yarr-lab-{uuid.uuid4().hex[:12]}"
        created = self.op(service, "post_qualityprofile", body=profile)
        identifier = created["id"]
        try:
            actual = self.http(service, f"/api/v3/qualityprofile/{identifier}")
            require(actual["name"] == profile["name"], "Create did not persist")
            actual["name"] += "-updated"
            self.op(service, "put_qualityprofile_by_id", id=str(identifier), body=actual)
            require(self.http(service, f"/api/v3/qualityprofile/{identifier}")["name"] == actual["name"], "Update did not persist")
            require(self.op(service, "get_qualityprofile_by_id", id=identifier)["name"] == actual["name"], "Read differs")
        finally:
            self.op(service, "delete_qualityprofile_by_id", id=identifier)
        require(all(item["id"] != identifier for item in self.http(service, "/api/v3/qualityprofile")), "Delete did not persist")
        return {"operations": ["get_qualityprofile", "post_qualityprofile", "get_qualityprofile_by_id",
                               "put_qualityprofile_by_id", "delete_qualityprofile_by_id"],
                "verified": "independent HTTP read-back after create, update, delete"}

    def tag_roundtrip(self, service):
        label = f"yarr-lab-{uuid.uuid4().hex[:12]}"
        created = self.op(service, "post_tag", body={"label": label})
        identifier = created["id"]
        try:
            require(self.http(service, f"/api/v3/tag/{identifier}")["label"] == label, "Tag create did not persist")
        finally:
            self.op(service, "delete_tag_by_id", id=identifier)
        require(all(item["id"] != identifier for item in self.http(service, "/api/v3/tag")), "Tag delete did not persist")
        return {"operations": ["post_tag", "delete_tag_by_id"], "verified": "create/delete read-back"}

    def invalid_request(self, service):
        message = self.cli(service, "op", "get_qualityprofile_by_id", "--args", '{"id":"not-an-integer"}', fail=True)
        require("schema" in message and "id" in message, message)
        return {"verified": "CLI reports parameter schema rejection", "operation": "get_qualityprofile_by_id"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["up", "status", "test", "down"])
    parser.add_argument("--binary", default=str(ROOT / "target/debug/yarr"))
    parser.add_argument("--report", type=Path, default=ROOT / ".cache/media-lab/report.json")
    parser.add_argument("--seed", action="store_true", help="Generate synthetic clips and verify library scans (requires ffmpeg)")
    args = parser.parse_args()
    if args.command in {"up", "down"}:
        compose_action(args.command, COMPOSE)
        return
    containers = inspect_lab()
    if args.command == "status":
        for service, info in containers.items():
            print(f"{service}: healthy, http://127.0.0.1:{PORTS[service][0]}, {info['Config']['Image']}")
        return
    report = {"started_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
              "scope": "isolated live smoke/CRUD; not exhaustive API coverage", "services": {}, "checks": []}
    with open(args.binary, "rb") as binary:
        report["binary_sha256"] = hashlib.file_digest(binary, "sha256").hexdigest()
    report.update(binary_path=str(Path(args.binary).resolve()), seed=args.seed,
                  git_revision=run("git", "-C", str(ROOT), "rev-parse", "HEAD").strip(),
                  source_dirty=bool(run("git", "-C", str(ROOT), "status", "--porcelain").strip()),
                  docker_engine_id=run("docker", "info", "--format", "{{.ID}}").strip())
    with tempfile.TemporaryDirectory(prefix="yarr-media-lab-") as home:
        lab = Lab(args.binary, home, containers)
        for service, info in containers.items():
            report["services"][service] = {"image": info["Config"]["Image"], "image_id": info["Image"]}
        checks = []
        from mcp_live import check_mcp
        for service in ("sonarr", "radarr"):
            checks.extend([(service, "status", lambda s=service: lab.cli(s, "status")),
                           (service, "quality_profile_crud", lambda s=service: lab.profile_roundtrip(s)),
                           (service, "tag_crud", lambda s=service: lab.tag_roundtrip(s)),
                           (service, "invalid_parameter", lambda s=service: lab.invalid_request(s)),
                           (service, "mcp_dispatch", lambda s=service: check_mcp(lab, s))])
            if args.seed:
                from media import seed_and_scan
                checks.append((service, "media_scan", lambda s=service: seed_and_scan(lab, s)))
        checks.extend([("plex", "identity", lambda: lab.op("plex", "get_identity")),
                       ("plex", "library_access", lambda: lab.http("plex", "/library/sections"))])
        for service, name, check in checks:
            result = {"service": service, "check": name}
            try:
                details = check()
                result["status"] = "passed"
                if name == "status":
                    report["services"][service]["version"] = details.get("version")
                elif name in {"identity", "library_access"}:
                    result["verified"] = "successful live response"
                else:
                    result["details"] = details
            except urllib.error.HTTPError as error:
                result.update(status="skipped" if service == "plex" and error.code in {401, 403} else "failed",
                              error=f"HTTP {error.code}: {error.reason}")
            except Exception as error:
                result.update(status="failed", error=lab.redact(str(error)))
            report["checks"].append(result)
            print(f"{service}/{name}: {result['status']}", flush=True)
    report["finished_at"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    report["outcome"] = ("failed" if any(check["status"] == "failed" for check in report["checks"])
                         else "incomplete" if any(check["status"] == "skipped" for check in report["checks"])
                         else "passed")
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(f"Report: {args.report}")
    if any(check["status"] == "failed" for check in report["checks"]):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
