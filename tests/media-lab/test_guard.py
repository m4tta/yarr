"""Unit coverage for the fail-closed media-lab Docker guard."""

import copy
import json
import os
from pathlib import Path
import subprocess
import unittest
from unittest import mock

import guard


def container_fixture(service="sonarr", *, running=True):
    host, internal = guard.PORTS[service]
    network_settings = {
        "Ports": {f"{internal}/tcp": [{"HostIp": "127.0.0.1", "HostPort": str(host)}]},
        "Networks": {f"{guard.PROJECT}_default": {}},
    }
    if not running:
        network_settings = {"Ports": {}, "Networks": {}}
    return {
        "Id": f"{service}-id",
        "Name": f"/{guard.PROJECT}-{service}-1",
        "Config": {
            "Labels": {
                "com.docker.compose.project": guard.PROJECT,
                "com.docker.compose.service": service,
            }
        },
        "State": {"Running": running, "Health": {"Status": "healthy" if running else "none"}},
        "HostConfig": {
            "PortBindings": {
                f"{internal}/tcp": [{"HostIp": "127.0.0.1", "HostPort": str(host)}]
            }
        },
        "NetworkSettings": network_settings,
        "Mounts": [
            {
                "Type": "volume",
                "Name": f"{guard.PROJECT}_{service}-config",
                "Destination": "/config",
                "RW": True,
            },
            {
                "Type": "volume",
                "Name": f"{guard.PROJECT}_media",
                "Destination": "/data",
                "RW": service != "plex",
            },
        ],
    }


def completed(args, *, stdout="", stderr="", returncode=0):
    return subprocess.CompletedProcess(args, returncode, stdout, stderr)


class ContainerGuardTests(unittest.TestCase):
    def test_running_and_stopped_lab_containers_are_valid_targets(self):
        guard.verify_container(container_fixture(), "sonarr")
        guard.verify_container(container_fixture(running=False), "sonarr", require_healthy=False)

    def test_stopped_container_uses_configured_host_binding(self):
        info = container_fixture(running=False)
        info["HostConfig"]["PortBindings"]["8989/tcp"][0]["HostIp"] = "0.0.0.0"
        with self.assertRaisesRegex(RuntimeError, "loopback"):
            guard.verify_container(info, "sonarr", require_healthy=False)

    def test_plex_media_volume_must_be_read_only(self):
        info = container_fixture("plex")
        info["Mounts"][1]["RW"] = True
        with self.assertRaisesRegex(RuntimeError, "read-only"):
            guard.verify_container(info, "plex")

    def test_running_container_must_use_only_lab_network(self):
        info = container_fixture()
        info["NetworkSettings"]["Networks"]["production_default"] = {}
        with self.assertRaisesRegex(RuntimeError, "isolated"):
            guard.verify_container(info, "sonarr")

    def test_foreign_volume_mount_is_refused(self):
        info = container_fixture()
        info["Mounts"][1]["Name"] = "production_media"
        with self.assertRaisesRegex(RuntimeError, "isolated lab volumes"):
            guard.verify_container(info, "sonarr")


class ComposeGuardTests(unittest.TestCase):
    def _empty_lab_runner(self, calls):
        def fake_run(args, **kwargs):
            calls.append((list(args), kwargs))
            if args[:3] == ["docker", "context", "inspect"]:
                payload = [{"Endpoints": {"docker": {"Host": "unix:///var/run/docker.sock"}}}]
                return completed(args, stdout=json.dumps(payload))
            if args[:3] in (["docker", "volume", "inspect"], ["docker", "container", "inspect"]):
                return completed(args, stderr="Error: No such object", returncode=1)
            if args[:3] == ["docker", "ps", "--all"]:
                return completed(args)
            if args[:2] == ["docker", "compose"]:
                return completed(args)
            raise AssertionError(f"Unexpected Docker invocation: {args}")

        return fake_run

    def test_compose_uses_explicit_project_and_drops_environment_override(self):
        calls = []
        with mock.patch.dict(os.environ, {"COMPOSE_PROJECT_NAME": "production"}), mock.patch.object(
            guard, "_run_process", side_effect=self._empty_lab_runner(calls)
        ):
            guard.compose_action("up", Path("compose.yaml"))

        compose_args, compose_options = next(
            (args, options) for args, options in calls if args[:2] == ["docker", "compose"]
        )
        self.assertEqual(
            compose_args[2:4], ["--project-name", guard.PROJECT]
        )
        self.assertNotIn("COMPOSE_PROJECT_NAME", compose_options["env"])
        self.assertEqual(compose_args[-5:], ["up", "-d", "--wait", "--wait-timeout", "240"])

    def test_foreign_existing_named_volume_blocks_compose(self):
        calls = []

        def fake_run(args, **kwargs):
            calls.append(list(args))
            if args[:3] == ["docker", "context", "inspect"]:
                payload = [{"Endpoints": {"docker": {"Host": "unix:///var/run/docker.sock"}}}]
                return completed(args, stdout=json.dumps(payload))
            if args[:3] == ["docker", "volume", "inspect"]:
                name = args[3]
                payload = [{
                    "Name": name,
                    "Labels": {
                        "com.docker.compose.project": "production",
                        "com.docker.compose.volume": "sonarr-config",
                    },
                }]
                return completed(args, stdout=json.dumps(payload))
            raise AssertionError(f"Mutation should be blocked before: {args}")

        with mock.patch.object(guard, "_run_process", side_effect=fake_run):
            with self.assertRaisesRegex(RuntimeError, "not owned"):
                guard.compose_action("down", Path("compose.yaml"))
        self.assertFalse(any(args[:2] == ["docker", "compose"] for args in calls))

    def test_foreign_existing_container_blocks_compose(self):
        calls = []
        foreign = container_fixture()
        foreign["Config"]["Labels"]["com.docker.compose.project"] = "production"

        def fake_run(args, **kwargs):
            calls.append(list(args))
            if args[:3] == ["docker", "context", "inspect"]:
                payload = [{"Endpoints": {"docker": {"Host": "unix:///var/run/docker.sock"}}}]
                return completed(args, stdout=json.dumps(payload))
            if args[:3] == ["docker", "volume", "inspect"]:
                return completed(args, stderr="Error: No such volume", returncode=1)
            if args[:3] == ["docker", "container", "inspect"]:
                if args[3] == f"{guard.PROJECT}-sonarr-1":
                    return completed(args, stdout=json.dumps([foreign]))
                return completed(args, stderr="Error: No such container", returncode=1)
            if args[:3] == ["docker", "ps", "--all"]:
                return completed(args)
            raise AssertionError(f"Mutation should be blocked before: {args}")

        with mock.patch.object(guard, "_run_process", side_effect=fake_run):
            with self.assertRaisesRegex(RuntimeError, "not owned"):
                guard.compose_action("down", Path("compose.yaml"))
        self.assertFalse(any(args[:2] == ["docker", "compose"] for args in calls))

    def test_remote_endpoint_blocks_compose_before_mutation(self):
        calls = []

        def fake_run(args, **kwargs):
            calls.append(list(args))
            payload = [{"Endpoints": {"docker": {"Host": "ssh://production.example"}}}]
            return completed(args, stdout=json.dumps(payload))

        with mock.patch.dict(os.environ, {}, clear=True), mock.patch.object(
            guard, "_run_process", side_effect=fake_run
        ):
            with self.assertRaisesRegex(RuntimeError, "remote contexts"):
                guard.compose_action("up", Path("compose.yaml"))
        self.assertEqual(calls, [["docker", "context", "inspect"]])

    def test_remote_context_cannot_be_hidden_by_local_docker_host(self):
        context = [{"Endpoints": {"docker": {"Host": "ssh://production.example"}}}]
        with mock.patch.dict(os.environ, {"DOCKER_CONTEXT": "production", "DOCKER_HOST": "unix:///var/run/docker.sock"}, clear=True), mock.patch.object(
            guard, "_run_process", return_value=completed([], stdout=json.dumps(context))
        ):
            with self.assertRaisesRegex(RuntimeError, "remote contexts"):
                guard.compose_action("down", Path("compose.yaml"))


if __name__ == "__main__":
    unittest.main()
