"""Unit tests for reversible media-lab administration checks."""

import copy
import unittest

from admin import check_admin


class FakeLab:
    def __init__(self, service, *, corrupt_custom_update=False, corrupt_naming_update=False):
        self.service = service
        self.calls = []
        self.custom_formats = {}
        self.next_id = 41
        self.corrupt_custom_update = corrupt_custom_update
        self.corrupt_naming_update = corrupt_naming_update
        field = "renameEpisodes" if service == "sonarr" else "renameMovies"
        self.naming = {
            "id": 1,
            field: False,
            "replaceIllegalCharacters": True,
            "format": "unchanged",
        }
        self.original_naming = copy.deepcopy(self.naming)

    def op(self, service, name, **args):
        if service != self.service:
            raise AssertionError("Wrong service")
        self.calls.append((name, copy.deepcopy(args)))

        if name == "post_customformat":
            resource = copy.deepcopy(args["body"])
            resource["id"] = self.next_id
            self.custom_formats[self.next_id] = resource
            return copy.deepcopy(resource)
        if name == "get_customformat_by_id":
            if not isinstance(args["id"], int):
                raise AssertionError("Custom-format GET id must be an integer")
            return copy.deepcopy(self.custom_formats[args["id"]])
        if name == "put_customformat_by_id":
            if not isinstance(args["id"], str):
                raise AssertionError("Custom-format PUT id must be a string")
            identifier = int(args["id"])
            self.custom_formats[identifier] = copy.deepcopy(args["body"])
            return copy.deepcopy(self.custom_formats[identifier])
        if name == "delete_customformat_by_id":
            if not isinstance(args["id"], int):
                raise AssertionError("Custom-format DELETE id must be an integer")
            self.custom_formats.pop(args["id"], None)
            return None
        if name == "get_config_naming":
            return copy.deepcopy(self.naming)
        if name == "get_config_naming_by_id":
            if not isinstance(args["id"], int):
                raise AssertionError("Naming GET id must be an integer")
            return copy.deepcopy(self.naming)
        if name == "put_config_naming_by_id":
            if not isinstance(args["id"], str):
                raise AssertionError("Naming PUT id must be a string")
            self.naming = copy.deepcopy(args["body"])
            return copy.deepcopy(self.naming)
        raise AssertionError(f"Unexpected operation: {name}")

    def http(self, service, path):
        if service != self.service:
            raise AssertionError("Wrong service")
        if path == "/api/v3/customformat":
            return [copy.deepcopy(item) for item in self.custom_formats.values()]
        if path.startswith("/api/v3/customformat/"):
            resource = copy.deepcopy(self.custom_formats[int(path.rsplit("/", 1)[1])])
            if self.corrupt_custom_update and resource["name"].endswith("-updated"):
                resource["name"] = "wrong"
            return resource
        if path == "/api/v3/config/naming/1":
            resource = copy.deepcopy(self.naming)
            changed = resource != self.original_naming
            if self.corrupt_naming_update and changed:
                field = "renameEpisodes" if service == "sonarr" else "renameMovies"
                resource[field] = self.original_naming[field]
            return resource
        raise AssertionError(f"Unexpected HTTP path: {path}")


class AdminChecksTests(unittest.TestCase):
    def test_sonarr_and_radarr_roundtrips_restore_all_state(self):
        for service in ("sonarr", "radarr"):
            with self.subTest(service=service):
                lab = FakeLab(service)
                details = check_admin(lab, service)
                self.assertEqual(lab.custom_formats, {})
                self.assertEqual(lab.naming, lab.original_naming)
                self.assertEqual(
                    details["naming_config"]["field"],
                    "renameEpisodes" if service == "sonarr" else "renameMovies",
                )
                put_calls = [args for name, args in lab.calls if name.startswith("put_")]
                self.assertTrue(put_calls)
                self.assertTrue(all(isinstance(args["id"], str) for args in put_calls))
                get_delete_calls = [
                    args
                    for name, args in lab.calls
                    if name in {"get_customformat_by_id", "delete_customformat_by_id", "get_config_naming_by_id"}
                ]
                self.assertTrue(all(isinstance(args["id"], int) for args in get_delete_calls))

    def test_custom_format_is_deleted_when_update_verification_fails(self):
        lab = FakeLab("sonarr", corrupt_custom_update=True)
        with self.assertRaisesRegex(AssertionError, "rename did not persist"):
            check_admin(lab, "sonarr")
        self.assertEqual(lab.custom_formats, {})

    def test_naming_config_is_restored_when_verification_fails(self):
        lab = FakeLab("radarr", corrupt_naming_update=True)
        with self.assertRaisesRegex(AssertionError, "did not persist"):
            check_admin(lab, "radarr")
        self.assertEqual(lab.custom_formats, {})
        self.assertEqual(lab.naming, lab.original_naming)

    def test_unknown_service_is_rejected_before_any_call(self):
        lab = FakeLab("sonarr")
        with self.assertRaisesRegex(ValueError, "only sonarr and radarr"):
            check_admin(lab, "plex")
        self.assertEqual(lab.calls, [])


if __name__ == "__main__":
    unittest.main()
