"""Plex fixture inventory supports either source and requires every marked file."""

import subprocess
import unittest
from unittest.mock import Mock, patch

from plex_media import _expected_files, _part_files


class PlexFixtureTests(unittest.TestCase):
    def test_either_fixture_source_is_sufficient(self):
        for marker in ("Lab", "Real"):
            paths = [f"/data/movies/Movie-Yarr{marker}0123456789ab.mkv",
                     f"/data/tv/Show-Yarr{marker}0123456789ab.mkv"]
            process = subprocess.CompletedProcess([], 0, stdout="\n".join(paths))
            with self.subTest(marker=marker), patch("plex_media.subprocess.run", return_value=process):
                expected = _expected_files(Mock())
                self.assertEqual(expected, {"movie": {paths[0]}, "show": {paths[1]}})

    def test_media_parts_are_collected_instead_of_counting_show_roots(self):
        items = [{"type": "show"}, {"Media": [{"Part": [{"file": "/a"}, {"file": "/b"}]}]},
                 {"Media": {"Part": {"file": "/c"}}}]
        self.assertEqual(_part_files(items), {"/a", "/b", "/c"})


if __name__ == "__main__":
    unittest.main()
