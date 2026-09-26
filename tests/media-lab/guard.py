"""Fail-closed Docker target checks for the disposable media lab."""

import json
import os
from pathlib import Path
import subprocess


PROJECT = "yarr-media-lab"
PORTS = {"sonarr": (18989, 8989), "radarr": (17878, 7878), "plex": (32401, 32400)}

_NETWORK = f"{PROJECT}_default"
_VOLUMES = {
    f"{PROJECT}_sonarr-config": "sonarr-config",
    f"{PROJECT}_radarr-config": "radarr-config",
    f"{PROJECT}_plex-config": "plex-config",
    f"{PROJECT}_media": "media",
}


def _run_process(args, *, check=True, capture_output=True, env=None):
    return subprocess.run(
        args,
        check=check,
        capture_output=capture_output,
        text=True,
        timeout=180,
        env=env,
    )


def _load_json(output, subject):
    try:
        return json.loads(output)
    except (TypeError, json.JSONDecodeError) as error:
        raise RuntimeError(f"Docker returned invalid JSON for {subject}") from error


def _local_docker_endpoint():
    result = _run_process(["docker", "context", "inspect"])
    contexts = _load_json(result.stdout, "the active context")
    if not isinstance(contexts, list) or len(contexts) != 1:
        raise RuntimeError("Expected exactly one active Docker context")
    try:
        context_endpoint = contexts[0]["Endpoints"]["docker"]["Host"]
    except (KeyError, TypeError) as error:
        raise RuntimeError("The active Docker context has no Docker endpoint") from error
    endpoint = context_endpoint if os.environ.get("DOCKER_CONTEXT") else os.environ.get("DOCKER_HOST") or context_endpoint
    if not isinstance(endpoint, str) or not endpoint.startswith("unix://"):
        raise RuntimeError(
            "Run on a local Linux Docker daemon (or inside WSL); remote contexts are refused"
        )
    return endpoint


def _optional_inspect(kind, name):
    result = _run_process(["docker", kind, "inspect", name], check=False)
    if result.returncode:
        message = (result.stderr or "").lower()
        if "no such" in message or "not found" in message:
            return None
        raise RuntimeError(f"Could not inspect Docker {kind} {name}: {(result.stderr or '').strip()}")
    values = _load_json(result.stdout, f"{kind} {name}")
    if not isinstance(values, list) or len(values) != 1:
        raise RuntimeError(f"Expected one Docker {kind} named {name}")
    return values[0]


def _container_is_running(info):
    return info.get("State", {}).get("Running") is True


def verify_container(info, service, *, require_healthy=True):
    """Reject foreign containers, storage, networking, and public bindings."""
    if service not in PORTS:
        raise RuntimeError(f"Unexpected media-lab service: {service}")

    labels = info.get("Config", {}).get("Labels") or {}
    if (
        labels.get("com.docker.compose.project") != PROJECT
        or labels.get("com.docker.compose.service") != service
    ):
        raise RuntimeError("Container is not owned by the media lab")

    expected_name = f"/{PROJECT}-{service}-1"
    if info.get("Name") not in {None, expected_name}:
        raise RuntimeError(f"Unexpected media-lab container name: {info['Name']}")

    running = _container_is_running(info)
    if require_healthy and not running:
        raise RuntimeError(f"{service} is not running")
    if require_healthy and info.get("State", {}).get("Health", {}).get("Status") != "healthy":
        raise RuntimeError(f"{service} is not healthy")

    host, internal = PORTS[service]
    if running:
        bindings = info.get("NetworkSettings", {}).get("Ports", {}).get(f"{internal}/tcp")
    else:
        bindings = info.get("HostConfig", {}).get("PortBindings", {}).get(f"{internal}/tcp")
    if bindings != [{"HostIp": "127.0.0.1", "HostPort": str(host)}]:
        raise RuntimeError("Expected an exclusive loopback port binding")

    mounts = info.get("Mounts") or []
    by_destination = {mount.get("Destination"): mount for mount in mounts}
    if set(by_destination) != {"/config", "/data"} or len(mounts) != 2:
        raise RuntimeError("Unexpected container mounts")
    expected_names = {
        "/config": f"{PROJECT}_{service}-config",
        "/data": f"{PROJECT}_media",
    }
    for destination, expected_name in expected_names.items():
        mount = by_destination[destination]
        if mount.get("Type") != "volume" or mount.get("Name") != expected_name:
            raise RuntimeError("Only isolated lab volumes are allowed")
        expected_writable = destination == "/config" or service != "plex"
        if mount.get("RW") is not expected_writable:
            mode = "writable" if expected_writable else "read-only"
            raise RuntimeError(f"Expected {service} {destination} to be {mode}")

    if running:
        networks = set((info.get("NetworkSettings", {}).get("Networks") or {}).keys())
        if networks != {_NETWORK}:
            raise RuntimeError(f"Expected only the isolated {_NETWORK} network")


def _validate_existing_volumes():
    for name, compose_key in _VOLUMES.items():
        info = _optional_inspect("volume", name)
        if info is None:
            continue
        labels = info.get("Labels") or {}
        if (
            info.get("Name") != name
            or labels.get("com.docker.compose.project") != PROJECT
            or labels.get("com.docker.compose.volume") != compose_key
        ):
            raise RuntimeError(f"Docker volume {name} is not owned by the media lab")


def _project_container_ids():
    result = _run_process(
        [
            "docker",
            "ps",
            "--all",
            "--quiet",
            "--filter",
            f"label=com.docker.compose.project={PROJECT}",
        ]
    )
    return {line.strip() for line in result.stdout.splitlines() if line.strip()}


def _validate_existing_containers():
    inspected = {}
    for service in PORTS:
        name = f"{PROJECT}-{service}-1"
        info = _optional_inspect("container", name)
        if info is not None:
            inspected[info.get("Id", name)] = info

    for container_id in _project_container_ids():
        if container_id in inspected:
            continue
        info = _optional_inspect("container", container_id)
        if info is None:
            raise RuntimeError(f"Media-lab container disappeared during validation: {container_id}")
        inspected[info.get("Id", container_id)] = info

    for info in inspected.values():
        service = (info.get("Config", {}).get("Labels") or {}).get(
            "com.docker.compose.service"
        )
        if service not in PORTS:
            raise RuntimeError(f"Unexpected container in the {PROJECT} project")
        verify_container(info, service, require_healthy=False)


def inspect_lab():
    """Return verified, healthy lab containers from the active local daemon."""
    _local_docker_endpoint()
    _validate_existing_volumes()
    result = {}
    for service in PORTS:
        name = f"{PROJECT}-{service}-1"
        info = _optional_inspect("container", name)
        if info is None:
            raise RuntimeError(f"Missing media-lab container: {name}")
        verify_container(info, service)
        result[service] = info
    return result


def compose_action(command, compose_path):
    """Run guarded lab Compose lifecycle actions against an explicit project."""
    if command not in {"up", "down"}:
        raise ValueError("Media-lab Compose action must be 'up' or 'down'")

    _local_docker_endpoint()
    _validate_existing_volumes()
    _validate_existing_containers()

    environment = os.environ.copy()
    environment.pop("COMPOSE_PROJECT_NAME", None)
    invocation = [
        "docker",
        "compose",
        "--project-name",
        PROJECT,
        "-f",
        str(Path(compose_path).resolve()),
        command,
    ]
    if command == "up":
        invocation.extend(["-d", "--wait", "--wait-timeout", "240"])
    _run_process(invocation, capture_output=False, env=environment)
