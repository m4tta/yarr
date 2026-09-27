import os
import pathlib
import unittest
from unittest import mock

import guard


class GuardTests(unittest.TestCase):
    def test_docker_is_pinned_to_local_socket_even_with_remote_context(self):
        with mock.patch.dict(os.environ, {"DOCKER_CONTEXT": "remote-production"}), mock.patch(
            "guard.subprocess.run"
        ) as run:
            guard._docker("version")
            args, kwargs = run.call_args
            self.assertEqual(args[0][:3], ["docker", "--host", "unix:///var/run/docker.sock"])
            self.assertNotIn("DOCKER_CONTEXT", kwargs["env"])

    def test_cleanup_does_not_remove_uninspected_orphans(self):
        with mock.patch("guard.inspect_lab"), mock.patch("guard._docker") as docker:
            docker.return_value.returncode = 0
            guard.compose_action("down", pathlib.Path(guard.__file__).with_name("compose.yaml"))
            self.assertNotIn("--remove-orphans", docker.call_args.args)

    def test_rejects_remote_docker_host(self):
        with mock.patch.dict(os.environ, {"DOCKER_HOST": "tcp://example.invalid:2376"}):
            with self.assertRaisesRegex(RuntimeError, "local Docker socket"):
                guard.inspect_lab()

    def test_rejects_other_compose_file(self):
        with self.assertRaisesRegex(RuntimeError, "unexpected Compose file"):
            guard.compose_action("up", pathlib.Path("elsewhere.yaml"))

    def test_rejects_unknown_lifecycle_action(self):
        with self.assertRaises(ValueError):
            guard.compose_action("restart", pathlib.Path("compose.yaml"))


if __name__ == "__main__":
    unittest.main()
