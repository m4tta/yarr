"""Reject incomplete or corrupt fixture copies before touching the media lab."""

import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock

from real_media import completed_command, load_manifest


class ManifestTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.file = Path(self.temporary.name, "fixture.mkv")
        self.file.write_bytes(b"local fixture bytes")
        fixture = {"file": str(self.file), "size": self.file.stat().st_size,
                   "sha256": hashlib.sha256(self.file.read_bytes()).hexdigest(), "metadataId": 1}
        self.manifest = {"version": 1, "radarr": dict(fixture),
                         "sonarr": dict(fixture, seasonNumber=1, episodeNumbers=[2])}
        self.path = Path(self.temporary.name, "manifest.json")

    def load(self):
        self.path.write_text(json.dumps(self.manifest), encoding="utf-8-sig")
        return load_manifest(self.path)

    def test_private_windows_bom_manifest_is_accepted(self):
        self.assertEqual(self.load()["sonarr"]["file"], str(self.file.resolve()))

    def test_same_size_corruption_is_rejected(self):
        self.file.write_bytes(b"Local fixture bytes")
        with self.assertRaisesRegex(AssertionError, "SHA-256"):
            self.load()

    def test_truncated_download_is_rejected(self):
        self.file.write_bytes(b"partial")
        with self.assertRaisesRegex(AssertionError, "size mismatch"):
            self.load()

    def test_ambiguous_or_invalid_episode_mapping_is_rejected(self):
        for numbers in ([], [True], [1, 1], [0], "2"):
            with self.subTest(numbers=numbers):
                self.manifest["sonarr"]["episodeNumbers"] = numbers
                with self.assertRaisesRegex(AssertionError, "episode numbers"):
                    self.load()


class CommandWaitTests(unittest.TestCase):
    def test_timed_out_import_is_resumed_without_resubmission(self):
        lab = Mock()
        lab.op.return_value = {"commandId": 7, "status": "timed_out", "finished": False}
        lab.http.return_value = {"id": 7, "status": "completed"}
        self.assertEqual(completed_command(lab, "sonarr", {"name": "ManualImport"}), 7)
        lab.op.assert_called_once()
        self.assertEqual(lab.http.call_count, 2)
        self.assertTrue(all(call.args[1] == "/api/v3/command/7" for call in lab.http.call_args_list))

    def test_failed_resumed_import_is_not_reported_as_success(self):
        lab = Mock()
        lab.op.return_value = {"commandId": 7, "status": "unknown", "finished": False}
        lab.http.return_value = {"id": 7, "status": "failed"}
        with self.assertRaisesRegex(AssertionError, "failed"):
            completed_command(lab, "radarr", {"name": "ManualImport"})
        lab.op.assert_called_once()


if __name__ == "__main__":
    unittest.main()
