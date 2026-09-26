"""Reversible live administration checks for disposable Sonarr/Radarr labs."""

import copy
import uuid


_SERVICES = {"sonarr": "renameEpisodes", "radarr": "renameMovies"}


def _require(condition, message):
    if not condition:
        raise AssertionError(message)


def _integer_id(resource, description):
    identifier = resource.get("id") if isinstance(resource, dict) else None
    _require(
        isinstance(identifier, int) and not isinstance(identifier, bool),
        f"{description} did not return an integer id",
    )
    return identifier


def _same_except(actual, expected, field):
    actual_rest = copy.deepcopy(actual)
    expected_rest = copy.deepcopy(expected)
    actual_rest.pop(field, None)
    expected_rest.pop(field, None)
    return actual_rest == expected_rest


def _has_release_title_specification(resource, pattern):
    specifications = resource.get("specifications") if isinstance(resource, dict) else None
    if not isinstance(specifications, list) or len(specifications) != 1:
        return False
    specification = specifications[0]
    if (
        specification.get("implementation") != "ReleaseTitleSpecification"
        or specification.get("negate") is not False
        or specification.get("required") is not False
    ):
        return False
    fields = specification.get("fields")
    return isinstance(fields, list) and any(
        field.get("name") == "value" and field.get("value") == pattern
        for field in fields
        if isinstance(field, dict)
    )


def _custom_format_roundtrip(lab, service):
    name = f"yarr-lab-{uuid.uuid4().hex[:12]}"
    updated_name = f"{name}-updated"
    pattern = f"{name}-release-title"
    identifier = None
    operations = [
        "post_customformat",
        "get_customformat_by_id",
        "put_customformat_by_id",
        "delete_customformat_by_id",
    ]

    try:
        created = lab.op(
            service,
            "post_customformat",
            body={
                "name": name,
                "includeCustomFormatWhenRenaming": False,
                "specifications": [
                    {
                        "name": "Yarr Lab Release Title",
                        "implementation": "ReleaseTitleSpecification",
                        "negate": False,
                        "required": False,
                        "fields": [{"name": "value", "value": pattern}],
                    }
                ],
            },
        )
        identifier = _integer_id(created, "Custom-format create")

        direct_created = lab.http(service, f"/api/v3/customformat/{identifier}")
        _require(direct_created.get("name") == name, "Custom-format create did not persist")
        _require(
            direct_created.get("includeCustomFormatWhenRenaming") is False,
            "Custom-format create changed the rename setting",
        )
        _require(
            _has_release_title_specification(direct_created, pattern),
            "Custom-format specification did not persist",
        )

        generated_created = lab.op(service, "get_customformat_by_id", id=identifier)
        _require(generated_created == direct_created, "Generated and direct custom-format reads differ")

        updated = copy.deepcopy(direct_created)
        updated["name"] = updated_name
        updated["includeCustomFormatWhenRenaming"] = True
        # The generated PUT path intentionally has a string id schema while the
        # generated GET/DELETE paths use integer ids.
        lab.op(service, "put_customformat_by_id", id=str(identifier), body=updated)

        direct_updated = lab.http(service, f"/api/v3/customformat/{identifier}")
        _require(direct_updated.get("name") == updated_name, "Custom-format rename did not persist")
        _require(
            direct_updated.get("includeCustomFormatWhenRenaming") is True,
            "Custom-format setting update did not persist",
        )
        _require(
            _has_release_title_specification(direct_updated, pattern),
            "Custom-format update changed its specification",
        )
        _require(
            lab.op(service, "get_customformat_by_id", id=identifier) == direct_updated,
            "Generated and direct custom-format reads differ after update",
        )
    finally:
        cleanup_ids = set()
        if identifier is not None:
            cleanup_ids.add(identifier)
        # Recover the id when an upstream create succeeds but its response is
        # malformed or response handling fails before we can retain the id.
        for item in lab.http(service, "/api/v3/customformat"):
            if item.get("name") in {name, updated_name}:
                candidate = item.get("id")
                if isinstance(candidate, int) and not isinstance(candidate, bool):
                    cleanup_ids.add(candidate)
        for cleanup_id in cleanup_ids:
            lab.op(service, "delete_customformat_by_id", id=cleanup_id)

    remaining = lab.http(service, "/api/v3/customformat")
    _require(
        all(item.get("id") != identifier for item in remaining),
        "Custom-format delete did not persist",
    )
    return {
        "operations": operations,
        "verified": "create/read/update/delete with independent HTTP read-back",
    }


def _naming_roundtrip(lab, service):
    field = _SERVICES[service]
    original = copy.deepcopy(lab.op(service, "get_config_naming"))
    identifier = _integer_id(original, "Naming-config read")
    _require(isinstance(original.get(field), bool), f"Naming config has no boolean {field}")

    changed = copy.deepcopy(original)
    changed[field] = not original[field]
    try:
        # The generated PUT path uses a string id schema.
        put_result = lab.op(
            service,
            "put_config_naming_by_id",
            id=str(identifier),
            body=changed,
        )
        _require(put_result.get(field) is changed[field], "Naming PUT returned the old flag")

        direct_changed = lab.http(service, f"/api/v3/config/naming/{identifier}")
        _require(direct_changed.get(field) is changed[field], "Naming flag update did not persist")
        _require(
            _same_except(direct_changed, original, field),
            "Naming update changed fields other than the selected flag",
        )
        generated_changed = lab.op(service, "get_config_naming_by_id", id=identifier)
        _require(generated_changed == direct_changed, "Generated and direct naming reads differ")
    finally:
        lab.op(
            service,
            "put_config_naming_by_id",
            id=str(identifier),
            body=original,
        )
        restored = lab.http(service, f"/api/v3/config/naming/{identifier}")
        _require(restored == original, "Naming config was not restored exactly")

    return {
        "field": field,
        "operations": [
            "get_config_naming",
            "put_config_naming_by_id",
            "get_config_naming_by_id",
        ],
        "verified": "one flag changed with no collateral fields, then exact restoration",
    }


def check_admin(lab, service):
    """Exercise reversible configuration administration on an isolated Arr service."""
    if service not in _SERVICES:
        raise ValueError("Admin checks support only sonarr and radarr")
    return {
        "custom_format": _custom_format_roundtrip(lab, service),
        "naming_config": _naming_roundtrip(lab, service),
    }
