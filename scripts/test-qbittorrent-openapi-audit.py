#!/usr/bin/env python3
"""Network-free negative tests for check-qbittorrent-openapi.py.

Run with: python3 scripts/test-qbittorrent-openapi-audit.py
"""

from __future__ import annotations

import copy
import importlib.util
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("check-qbittorrent-openapi.py")
MODULE_SPEC = importlib.util.spec_from_file_location("qbittorrent_openapi_audit", SCRIPT)
if MODULE_SPEC is None or MODULE_SPEC.loader is None:
    raise RuntimeError(f"cannot load {SCRIPT}")
AUDIT = importlib.util.module_from_spec(MODULE_SPEC)
MODULE_SPEC.loader.exec_module(AUDIT)


def fixture() -> tuple[dict, dict, dict, set[str], set[str]]:
    operation = {
        "operationId": "post_torrents_recheck",
        "requestBody": {
            "required": True,
            "content": {
                "application/x-www-form-urlencoded": {
                    "schema": {
                        "type": "object",
                        "additionalProperties": False,
                        "required": ["hashes"],
                        "properties": {"hashes": {"type": "string", "minLength": 1}},
                    }
                }
            },
        },
        "responses": {"200": {"description": "Success"}},
    }
    spec = {
        "openapi": "3.0.3",
        "servers": [{"url": "/api/v2"}],
        "security": [{"SID": []}],
        "components": {
            "securitySchemes": {
                "SID": {"type": "apiKey", "in": "cookie", "name": "SID"},
            }
        },
        "paths": {"/torrents/recheck": {"post": operation}},
    }
    coverage = {
        "upstream": {"commit": "0b63c3d17373f6132ea211c9dcd4241284ccdfaf"},
        "inventory": [
            {
                "path": "/torrents/recheck",
                "method": "post",
                "operationId": "post_torrents_recheck",
            }
        ],
        "exclusions": [
            {"path": "/auth/login", "reason": "transport managed"},
            {"path": "/auth/logout", "reason": "transport managed"},
        ],
    }
    source = {
        "/torrents/recheck": {"params": {"hashes"}, "required": {"hashes"}},
        "/auth/login": {"params": {"username", "password"}, "required": set()},
        "/auth/logout": {"params": set(), "required": set()},
    }
    return spec, coverage, source, {"/torrents/recheck"}, {"/torrents/recheck"}


class AuditNegativeTests(unittest.TestCase):
    def assert_rejected(self, spec: dict, coverage: dict, source: dict, forced_post: set[str], wiki: set[str], text: str) -> None:
        with self.assertRaisesRegex(AUDIT.AuditFailure, text):
            AUDIT.audit(spec, coverage, source, forced_post, wiki)

    def test_fixture_is_valid(self) -> None:
        AUDIT.audit(*fixture())

    def test_missing_operation_is_rejected(self) -> None:
        spec, coverage, source, forced_post, wiki = fixture()
        spec["paths"] = {}
        self.assert_rejected(spec, coverage, source, forced_post, wiki, "missing from spec")

    def test_wrong_http_method_is_rejected(self) -> None:
        spec, coverage, source, forced_post, wiki = fixture()
        operation = spec["paths"]["/torrents/recheck"].pop("post")
        operation.pop("requestBody")
        operation["parameters"] = [
            {"name": "hashes", "in": "query", "required": True, "schema": {"type": "string"}}
        ]
        spec["paths"]["/torrents/recheck"]["get"] = operation
        coverage["inventory"][0]["method"] = "get"
        self.assert_rejected(spec, coverage, source, forced_post, wiki, "requires POST")

    def test_dropped_required_field_is_rejected(self) -> None:
        spec, coverage, source, forced_post, wiki = fixture()
        schema = spec["paths"]["/torrents/recheck"]["post"]["requestBody"]["content"]["application/x-www-form-urlencoded"]["schema"]
        schema["required"] = []
        self.assert_rejected(spec, coverage, source, forced_post, wiki, "source-required parameters are optional")

    def test_impossible_enum_is_rejected(self) -> None:
        spec, coverage, source, forced_post, wiki = fixture()
        schema = spec["paths"]["/torrents/recheck"]["post"]["requestBody"]["content"]["application/x-www-form-urlencoded"]["schema"]
        schema["properties"]["hashes"] = {"type": "integer", "enum": ["all"]}
        self.assert_rejected(spec, coverage, source, forced_post, wiki, "do not match schema type")


if __name__ == "__main__":
    unittest.main()
