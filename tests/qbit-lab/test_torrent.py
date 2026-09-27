import hashlib
import tempfile
import unittest
from pathlib import Path

from torrent import bencode, create_fixture


class TorrentFixtureTests(unittest.TestCase):
    def test_bencode_orders_dictionary_keys(self):
        self.assertEqual(bencode({b"z": 1, b"a": b"x"}), b"d1:a1:x1:zi1ee")

    def test_fixture_is_deterministic_and_tracker_free(self):
        with tempfile.TemporaryDirectory() as root:
            payload, meta, info_hash = create_fixture(Path(root), "fixed")
            encoded = meta.read_bytes()
            self.assertNotIn(b"announce", encoded)
            self.assertIn(payload.name.encode(), encoded)
            self.assertEqual(len(info_hash), 40)
            self.assertNotEqual(hashlib.sha1(payload.read_bytes()).hexdigest(), info_hash)


if __name__ == "__main__":
    unittest.main()
