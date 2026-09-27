"""Docker ownership and endpoint guards for the disposable qBittorrent lab."""

from __future__ import annotations

import json
import os
import pathlib
import subprocess

PROJECT = "yarr-qbit-lab"
SERVICE = "qbittorrent"
CONTAINER = f"{PROJECT}-{SERVICE}-1"
PORT = 18083
IMAGE = (
    "ghcr.io/linuxserver/qbittorrent:version-5.2.3_v2.0.15@"
    "sha256:0ec64d5c7b7ba07c9c28458c78306231ac7677efb9ce8198f3dcb2e27cb2d444"
)
VOLUMES = {
    f"{PROJECT}_qbit-config": "/config",
    f"{PROJECT}_qbit-downloads": "/downloads",
}
NETWORK = f"{PROJECT}-network"


def _docker(*args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
    env = os.environ.copy()
    for name in ("COMPOSE_PROJECT_NAME", "COMPOSE_FILE", "DOCKER_CONTEXT"):
        env.pop(name, None)
    return subprocess.run(
        ["docker", "--host", "unix:///var/run/docker.sock", *args], check=check, capture_output=True, text=True, env=env
    )


def _inspect(kind: str, name: str) -> dict | None:
    result = _docker(kind, "inspect", name, check=False)
    if result.returncode:
        return None
    values = json.loads(result.stdout)
    if len(values) != 1:
        raise RuntimeError(f"unexpected Docker inspection result for {name}")
    return values[0]


def _labels(info: dict) -> dict:
    return info.get("Labels") or info.get("Config", {}).get("Labels") or {}


def _owned(info: dict, resource: str) -> None:
    labels = _labels(info)
    if labels.get("com.docker.compose.project") != PROJECT:
        raise RuntimeError(f"refusing to touch unowned Docker resource {resource}")


def inspect_lab(require_running: bool = False) -> dict | None:
    """Validate every pre-existing lab resource and return container inspection."""
    if os.environ.get("DOCKER_HOST") or os.environ.get("DOCKER_TLS_VERIFY"):
        raise RuntimeError("qBittorrent lab requires the local Docker socket")

    info = _inspect("container", CONTAINER)
    if info is not None:
        _owned(info, CONTAINER)
        labels = _labels(info)
        if labels.get("com.docker.compose.service") != SERVICE:
            raise RuntimeError(f"unexpected Compose service on {CONTAINER}")
        if info.get("Config", {}).get("Image") != IMAGE:
            raise RuntimeError(f"unexpected image on {CONTAINER}")
        mounts = {
            mount.get("Name"): mount.get("Destination")
            for mount in info.get("Mounts", [])
            if mount.get("Type") == "volume"
        }
        if mounts != VOLUMES or any(
            mount.get("Type") != "volume" for mount in info.get("Mounts", [])
        ):
            raise RuntimeError(f"unexpected mounts on {CONTAINER}")
        networks = set(info.get("NetworkSettings", {}).get("Networks", {}))
        if networks != {NETWORK}:
            raise RuntimeError(f"unexpected networks on {CONTAINER}")
        bindings = info.get("HostConfig", {}).get("PortBindings") or {}
        expected = {"8080/tcp": [{"HostIp": "127.0.0.1", "HostPort": str(PORT)}]}
        if bindings != expected:
            raise RuntimeError(f"unexpected host port binding on {CONTAINER}")
        if require_running and not info.get("State", {}).get("Running"):
            raise RuntimeError("qBittorrent lab container is not running")

    for name in VOLUMES:
        volume = _inspect("volume", name)
        if volume is not None:
            _owned(volume, name)
            if _labels(volume).get("com.docker.compose.volume") != name.removeprefix(
                f"{PROJECT}_"
            ):
                raise RuntimeError(f"unexpected Compose volume label on {name}")

    network = _inspect("network", NETWORK)
    if network is not None:
        _owned(network, NETWORK)

    return info


def compose_action(command: str, compose_path: pathlib.Path) -> None:
    """Run the only allowed Compose lifecycle operations after ownership checks."""
    if command not in {"up", "down"}:
        raise ValueError("compose command must be up or down")
    expected = pathlib.Path(__file__).with_name("compose.yaml").resolve()
    if compose_path.resolve() != expected:
        raise RuntimeError("refusing an unexpected Compose file")
    inspect_lab()
    args = [
        "compose",
        "--project-name",
        PROJECT,
        "--file",
        str(expected),
    ]
    if command == "up":
        args += ["up", "--detach", "--wait"]
    else:
        args += ["down", "--volumes"]
    result = _docker(*args, check=False)
    if result.returncode:
        message = (result.stderr or result.stdout).strip().splitlines()[-1:]
        raise RuntimeError(message[0] if message else f"docker compose {command} failed")
    if command == "up":
        inspect_lab(require_running=True)
