#!/usr/bin/env python3
"""Audit the pinned qBittorrent Web API source census against the local spec."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
import urllib.request
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_SPEC = ROOT / "specs/qbittorrent.openapi.json"
DEFAULT_COVERAGE = ROOT / "specs/qbittorrent-coverage.json"

CONTROLLERS = {
    "app": "appcontroller",
    "auth": "authcontroller",
    "clientdata": "clientdatacontroller",
    "log": "logcontroller",
    "rss": "rsscontroller",
    "search": "searchcontroller",
    "sync": "synccontroller",
    "torrentcreator": "torrentcreatorcontroller",
    "torrents": "torrentscontroller",
    "transfer": "transfercontroller",
}

ACTION_RE = re.compile(r"\bvoid\s+[A-Za-z0-9_]+::([A-Za-z0-9_]+)Action\s*\(\s*\)\s*\{")
HEADER_ACTION_RE = re.compile(r"\bvoid\s+([A-Za-z0-9_]+)Action\s*\(\s*\)\s*;")
CONSTANT_RE = re.compile(r'\b(?:const\s+QString|constexpr\s+auto)\s+([A-Z][A-Z0-9_]*)\s*=\s*u?"([^"]+)"(?:_s)?\s*;')
LITERAL = r'u?"([^"]+)"(?:_s)?'


class AuditFailure(RuntimeError):
    pass


def load_json(path: Path) -> dict:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise AuditFailure(f"cannot read {path}: {exc}") from exc


def fetch_text(repository: str, commit: str, relative_path: str) -> str:
    owner_repo = repository.removeprefix("https://github.com/").removesuffix(".git")
    url = f"https://raw.githubusercontent.com/{owner_repo}/{commit}/{relative_path}"
    try:
        with urllib.request.urlopen(url, timeout=30) as response:
            return response.read().decode("utf-8")
    except Exception as exc:  # urllib wraps DNS, TLS, HTTP, and timeout failures differently
        raise AuditFailure(f"cannot fetch pinned source {url}: {exc}") from exc


def source_texts(coverage: dict, source_root: Path | None) -> dict[str, str]:
    upstream = coverage["upstream"]
    repository = upstream["repository"]
    commit = upstream["commit"]
    paths = {"src/webui/webapplication.h", "src/webui/webapplication.cpp"}
    for stem in CONTROLLERS.values():
        paths.add(f"src/webui/api/{stem}.h")
        paths.add(f"src/webui/api/{stem}.cpp")

    texts: dict[str, str] = {}
    for relative in sorted(paths):
        if source_root is None:
            texts[relative] = fetch_text(repository, commit, relative)
        else:
            path = source_root / relative
            try:
                texts[relative] = path.read_text(encoding="utf-8")
            except OSError as exc:
                raise AuditFailure(f"cannot read pinned source file {path}: {exc}") from exc
    return texts


def wiki_contract(coverage: dict, wiki_file: Path | None) -> set[str]:
    wiki = coverage["upstream"]["wiki"]
    if wiki_file is None:
        text = fetch_text(wiki["repository"], wiki["commit"], wiki["path"])
    else:
        try:
            text = wiki_file.read_text(encoding="utf-8")
        except OSError as exc:
            raise AuditFailure(f"cannot read pinned wiki file {wiki_file}: {exc}") from exc
    digest = hashlib.sha256(text.encode("utf-8")).hexdigest()
    if digest != wiki["sha256"]:
        raise AuditFailure(
            f"pinned wiki checksum mismatch: expected {wiki['sha256']}, found {digest}"
        )
    scopes = set(CONTROLLERS)
    return {
        f"/{scope}/{action}"
        for scope, action in re.findall(r"/api/v2/([A-Za-z_][A-Za-z_0-9]*)/([A-Za-z_][A-Za-z_0-9]*)", text)
        if scope in scopes and action != "methodName"
    }


def function_body(text: str, opening_brace: int) -> str:
    depth = 0
    state = "code"
    index = opening_brace
    while index < len(text):
        char = text[index]
        next_char = text[index + 1] if index + 1 < len(text) else ""
        if state == "code":
            if char == '"':
                state = "string"
            elif char == "'":
                state = "char"
            elif char == "/" and next_char == "/":
                state = "line_comment"
                index += 1
            elif char == "/" and next_char == "*":
                state = "block_comment"
                index += 1
            elif char == "{":
                depth += 1
            elif char == "}":
                depth -= 1
                if depth == 0:
                    return text[opening_brace + 1 : index]
        elif state == "string":
            if char == "\\":
                index += 1
            elif char == '"':
                state = "code"
        elif state == "char":
            if char == "\\":
                index += 1
            elif char == "'":
                state = "code"
        elif state == "line_comment":
            if char == "\n":
                state = "code"
        elif state == "block_comment" and char == "*" and next_char == "/":
            state = "code"
            index += 1
        index += 1
    raise AuditFailure("unbalanced action body in pinned qBittorrent source")


def resolve_token(token: str, constants: dict[str, str]) -> str | None:
    literal = re.fullmatch(LITERAL, token.strip())
    if literal:
        return literal.group(1)
    return constants.get(token.strip())


def action_contracts(scope: str, cpp: str) -> dict[str, dict[str, set[str]]]:
    constants = dict(CONSTANT_RE.findall(cpp))
    contracts: dict[str, dict[str, set[str]]] = {}
    access_patterns = [
        re.compile(rf"params\s*\(\s*\)\s*\[\s*({LITERAL}|[A-Z][A-Z0-9_]*)\s*\]"),
        re.compile(rf"params\s*\(\s*\)\s*\.\s*(?:value|constFind)\s*\(\s*({LITERAL}|[A-Z][A-Z0-9_]*)"),
        re.compile(rf"getOptional[A-Za-z0-9_]*\s*\(\s*params\s*\(\s*\)\s*,\s*({LITERAL}|[A-Z][A-Z0-9_]*)"),
    ]
    for match in ACTION_RE.finditer(cpp):
        action = match.group(1)
        body = function_body(cpp, match.end() - 1)
        params: set[str] = set()
        required: set[str] = set()
        for pattern in access_patterns:
            for access in pattern.finditer(body):
                token = access.group(1)
                if name := resolve_token(token, constants):
                    params.add(name)
        for required_call in re.finditer(r"requireParams\s*\(\s*\{(.*?)\}\s*\)", body, re.S):
            for token in required_call.group(1).split(","):
                if name := resolve_token(token, constants):
                    required.add(name)
                    params.add(name)
        contracts[f"/{scope}/{action}"] = {"params": params, "required": required}
    return contracts


def source_census(texts: dict[str, str]) -> tuple[dict[str, dict[str, set[str]]], set[str]]:
    webapplication = texts["src/webui/webapplication.cpp"]
    registered_scopes = set(
        re.findall(r'registerAPIController\s*\(\s*u"([^"]+)"_s', webapplication)
    )
    if re.search(r"m_authController\s*\{\s*new\s+AuthController\b", webapplication):
        registered_scopes.add("auth")
    known_scopes = set(CONTROLLERS)
    if unknown := registered_scopes - known_scopes:
        raise AuditFailure(
            f"pinned source registers unknown API controller scopes: {sorted(unknown)}"
        )
    if missing := known_scopes - registered_scopes:
        raise AuditFailure(
            f"known API controller scopes are no longer registered: {sorted(missing)}"
        )

    routes: dict[str, dict[str, set[str]]] = {}
    for scope, stem in CONTROLLERS.items():
        header_path = f"src/webui/api/{stem}.h"
        cpp_path = f"src/webui/api/{stem}.cpp"
        declared = {f"/{scope}/{action}" for action in HEADER_ACTION_RE.findall(texts[header_path])}
        contracts = action_contracts(scope, texts[cpp_path])
        missing_definitions = declared - set(contracts)
        extra_definitions = set(contracts) - declared
        if missing_definitions or extra_definitions:
            raise AuditFailure(
                f"controller declaration/definition drift: missing={sorted(missing_definitions)}, "
                f"extra={sorted(extra_definitions)}"
            )
        routes.update(contracts)

    forced_post = {
        f"/{scope}/{action}"
        for scope, action in re.findall(
            r'\{\{u"([^"]+)"_s,\s*u"([^"]+)"_s\},\s*Http::METHOD_POST\}',
            texts["src/webui/webapplication.h"],
        )
    }
    unknown_forced = forced_post - set(routes)
    if unknown_forced:
        raise AuditFailure(f"HTTP method table names unknown routes: {sorted(unknown_forced)}")
    return routes, forced_post


def spec_operations(spec: dict) -> dict[str, dict]:
    operations: dict[str, dict] = {}
    for path, path_item in spec.get("paths", {}).items():
        for method in ("get", "post"):
            if operation := path_item.get(method):
                key = f"{method.upper()} {path}"
                if key in operations:
                    raise AuditFailure(f"duplicate spec operation {key}")
                operations[key] = operation
    return operations


def operation_params(operation: dict, method: str) -> tuple[set[str], set[str]]:
    if method == "GET":
        params = {
            parameter["name"]
            for parameter in operation.get("parameters", [])
            if parameter.get("in") == "query"
        }
        required = {
            parameter["name"]
            for parameter in operation.get("parameters", [])
            if parameter.get("in") == "query" and parameter.get("required") is True
        }
        return params, required

    content = operation.get("requestBody", {}).get("content", {})
    if not content:
        return set(), set()
    unsupported = set(content) - {"application/x-www-form-urlencoded", "multipart/form-data"}
    if unsupported:
        raise AuditFailure(f"POST operation uses unsupported request media types: {sorted(unsupported)}")
    media = next(iter(content.values()))
    schema = media.get("schema", {})
    return set(schema.get("properties", {})), set(schema.get("required", []))


def enum_type_failures(value: object, location: str = "spec") -> list[str]:
    failures: list[str] = []
    if isinstance(value, dict):
        schema_type = value.get("type")
        if "enum" in value and schema_type:
            predicates = {
                "string": lambda item: isinstance(item, str),
                "integer": lambda item: isinstance(item, int) and not isinstance(item, bool),
                "number": lambda item: isinstance(item, (int, float)) and not isinstance(item, bool),
                "boolean": lambda item: isinstance(item, bool),
            }
            predicate = predicates.get(schema_type)
            if predicate and any(not predicate(item) for item in value["enum"]):
                failures.append(
                    f"{location}: enum values {value['enum']!r} do not match schema type {schema_type!r}"
                )
        for key, child in value.items():
            failures.extend(enum_type_failures(child, f"{location}.{key}"))
    elif isinstance(value, list):
        for index, child in enumerate(value):
            failures.extend(enum_type_failures(child, f"{location}[{index}]"))
    return failures


def audit(
    spec: dict,
    coverage: dict,
    source: dict[str, dict[str, set[str]]],
    forced_post: set[str],
    wiki_routes: set[str],
) -> None:
    failures: list[str] = enum_type_failures(spec)
    inventory = coverage.get("inventory", [])
    exclusions = coverage.get("exclusions", [])

    mapped_paths = [row["path"] for row in inventory]
    excluded_paths = [row["path"] for row in exclusions]
    if len(mapped_paths) != len(set(mapped_paths)):
        failures.append("coverage inventory maps a source route more than once")
    if len(excluded_paths) != len(set(excluded_paths)):
        failures.append("coverage exclusions name a source route more than once")
    overlap = set(mapped_paths) & set(excluded_paths)
    if overlap:
        failures.append(f"routes are both mapped and excluded: {sorted(overlap)}")

    source_paths = set(source)
    covered_paths = set(mapped_paths) | set(excluded_paths)
    if missing := source_paths - covered_paths:
        failures.append(f"source routes missing coverage decisions: {sorted(missing)}")
    if extra := covered_paths - source_paths:
        failures.append(f"coverage names routes absent from pinned source: {sorted(extra)}")
    if missing := wiki_routes - source_paths:
        failures.append(f"official pinned wiki names routes absent from 5.2.3 source: {sorted(missing)}")
    if missing := wiki_routes - covered_paths:
        failures.append(f"official pinned wiki routes lack coverage decisions: {sorted(missing)}")

    operations = spec_operations(spec)
    expected_operations = {f"{row['method'].upper()} {row['path']}" for row in inventory}
    if missing := expected_operations - set(operations):
        failures.append(f"coverage operations missing from spec: {sorted(missing)}")
    if extra := set(operations) - expected_operations:
        failures.append(f"spec operations missing from coverage inventory: {sorted(extra)}")

    operation_ids: set[str] = set()
    for row in inventory:
        key = f"{row['method'].upper()} {row['path']}"
        operation = operations.get(key)
        if not operation:
            continue
        operation_id = operation.get("operationId")
        if operation_id != row.get("operationId"):
            failures.append(
                f"{key}: operationId {operation_id!r} does not match coverage {row.get('operationId')!r}"
            )
        if operation_id in operation_ids:
            failures.append(f"duplicate operationId {operation_id!r}")
        operation_ids.add(operation_id)

        if row["path"] in forced_post and row["method"] != "post":
            failures.append(f"{row['path']}: pinned source requires POST")
        actual_params, actual_required = operation_params(operation, row["method"].upper())
        detected = source[row["path"]]
        if missing := detected["params"] - actual_params:
            failures.append(f"{key}: missing source-observed parameters {sorted(missing)}")
        allowed_synthetic = {
            "/torrents/add": {"torrents"},
            "/torrents/parseMetadata": {"torrent"},
        }.get(row["path"], set())
        if extra := actual_params - detected["params"] - allowed_synthetic:
            failures.append(f"{key}: parameters not observed in pinned source {sorted(extra)}")
        if missing := detected["required"] - actual_required:
            failures.append(f"{key}: source-required parameters are optional in spec {sorted(missing)}")
        if row["method"] == "get" and operation.get("requestBody"):
            failures.append(f"{key}: GET must express inputs as query parameters")
        if row["method"] == "post" and operation.get("parameters"):
            query_names = [p.get("name") for p in operation["parameters"] if p.get("in") == "query"]
            if query_names:
                failures.append(f"{key}: POST inputs must be form fields, found query {query_names}")
        if row["method"] == "post" and operation.get("requestBody"):
            media = next(iter(operation["requestBody"]["content"].values()))
            if media.get("schema", {}).get("additionalProperties") is not False:
                failures.append(f"{key}: request object must reject unknown fields")

    allowed_exclusions = {"/auth/login", "/auth/logout"}
    if set(excluded_paths) != allowed_exclusions:
        failures.append(
            "only transport-managed auth login/logout may be excluded; "
            f"found {sorted(excluded_paths)}"
        )
    for exclusion in exclusions:
        if not exclusion.get("reason"):
            failures.append(f"{exclusion['path']}: exclusion has no reason")

    if spec.get("openapi") != "3.0.3":
        failures.append("spec must use OpenAPI 3.0.3")
    servers = spec.get("servers", [])
    if not servers or servers[0].get("url") != "/api/v2":
        failures.append("spec server URL must be /api/v2")
    if coverage.get("upstream", {}).get("commit") != "0b63c3d17373f6132ea211c9dcd4241284ccdfaf":
        failures.append("coverage must stay pinned to the official qBittorrent 5.2.3 commit")
    security_schemes = spec.get("components", {}).get("securitySchemes", {})
    if set(security_schemes) != {"SID"}:
        failures.append("spec must advertise only transport-managed SID cookie authentication")
    sid = security_schemes.get("SID", {})
    if (sid.get("type"), sid.get("in"), sid.get("name")) != ("apiKey", "cookie", "SID"):
        failures.append("SID security scheme must be the upstream SID cookie")
    if spec.get("security") != [{"SID": []}]:
        failures.append("global security must require only the SID cookie scheme")
    numeric_text_operations = {
        "GET /torrents/count",
        "GET /transfer/downloadLimit",
        "GET /transfer/speedLimitsMode",
        "GET /transfer/uploadLimit",
    }
    for key in numeric_text_operations & set(operations):
        schema = (
            operations[key]
            .get("responses", {})
            .get("200", {})
            .get("content", {})
            .get("text/plain", {})
            .get("schema", {})
        )
        if schema.get("type") != "string":
            failures.append(f"{key}: plaintext numeric response must use a string wire schema")
        if "number" not in schema.get("description", "").lower():
            failures.append(f"{key}: plaintext numeric response must explain numeric parsing")
    if "POST /torrents/add" in operations:
        torrent_add = operations["POST /torrents/add"].get("requestBody", {}).get("content", {}).get("multipart/form-data", {}).get("schema", {})
        if torrent_add.get("anyOf") != [{"required": ["urls"]}, {"required": ["torrents"]}]:
            failures.append("POST /torrents/add must require either `urls` or uploaded `torrents`")

    if failures:
        raise AuditFailure("\n".join(f"- {failure}" for failure in failures))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--spec", type=Path, default=DEFAULT_SPEC)
    parser.add_argument("--coverage", type=Path, default=DEFAULT_COVERAGE)
    parser.add_argument(
        "--source-root",
        type=Path,
        help="qBittorrent 5.2.3 checkout; omitted downloads the pinned official files",
    )
    parser.add_argument(
        "--wiki-file",
        type=Path,
        help="pinned official wiki Markdown; omitted downloads and checksum-verifies it",
    )
    parser.add_argument(
        "--print-source-census",
        action="store_true",
        help="print detected routes and parameters without loading the local spec",
    )
    args = parser.parse_args()

    try:
        coverage = load_json(args.coverage)
        texts = source_texts(coverage, args.source_root)
        source, forced_post = source_census(texts)
        wiki_routes = wiki_contract(coverage, args.wiki_file)
        if args.print_source_census:
            for path in sorted(source):
                contract = source[path]
                print(
                    "\t".join(
                        [
                            path,
                            "POST" if path in forced_post else "GET|POST",
                            ",".join(sorted(contract["required"])),
                            ",".join(sorted(contract["params"] - contract["required"])),
                        ]
                    )
                )
            return 0
        spec = load_json(args.spec)
        audit(spec, coverage, source, forced_post, wiki_routes)
    except AuditFailure as exc:
        print(f"qBittorrent OpenAPI audit failed:\n{exc}", file=sys.stderr)
        return 1

    print(
        f"qBittorrent OpenAPI audit passed: {len(source)} source routes, "
        f"{len(coverage['inventory'])} specified, {len(coverage['exclusions'])} excluded"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
