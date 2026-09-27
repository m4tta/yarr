#!/usr/bin/env python3
"""Generate the locally maintained qBittorrent 5.2.3 OpenAPI contract."""

from __future__ import annotations

import json
import re
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC_PATH = ROOT / "specs/qbittorrent.openapi.json"
COVERAGE_PATH = ROOT / "specs/qbittorrent-coverage.json"

UPSTREAM = {
    "repository": "https://github.com/qbittorrent/qBittorrent",
    "ref": "release-5.2.3",
    "commit": "0b63c3d17373f6132ea211c9dcd4241284ccdfaf",
    "apiBase": "/api/v2",
    "wiki": {
        "repository": "https://github.com/qbittorrent/wiki",
        "commit": "74b1c38558c3ed831c1fd895946882753835cbc7",
        "path": "WebUI-API-(qBittorrent-5.0).md",
        "sha256": "b9088dc7805c4ecb43901f18079d78ade78336d3647954f410a1b429d4902a62",
    },
}

# route -> (required parameters, optional parameters, response schema key)
GET_ROUTES = {
    "/app/buildInfo": ((), (), "BuildInfo"),
    "/app/cookies": ((), (), "CookieList"),
    "/app/defaultSavePath": ((), (), "Text"),
    "/app/getDirectoryContent": (("dirPath",), ("mode", "withMetadata"), "DirectoryContents"),
    "/app/networkInterfaceAddressList": (("iface",), (), "StringList"),
    "/app/networkInterfaceList": ((), (), "NetworkInterfaceList"),
    "/app/preferences": ((), (), "DynamicObject"),
    "/app/processInfo": ((), (), "ProcessInfo"),
    "/app/version": ((), (), "Text"),
    "/app/webapiVersion": ((), (), "Text"),
    "/clientdata/load": ((), ("keys",), "DynamicObject"),
    "/log/main": ((), ("normal", "info", "warning", "critical", "last_known_id"), "LogEntryList"),
    "/log/peers": ((), ("last_known_id",), "PeerLogEntryList"),
    "/rss/items": ((), ("withData",), "DynamicObject"),
    "/rss/matchingArticles": (("ruleName",), (), "DynamicObject"),
    "/rss/rules": ((), (), "DynamicObject"),
    "/search/plugins": ((), (), "SearchPluginList"),
    "/search/results": (("id",), ("limit", "offset"), "SearchResults"),
    "/search/status": ((), ("id",), "SearchStatusList"),
    "/sync/maindata": ((), ("rid",), "DynamicObject"),
    "/sync/torrentPeers": (("hash",), ("rid",), "DynamicObject"),
    "/torrentcreator/status": ((), ("taskID",), "TorrentCreatorTaskList"),
    "/torrentcreator/torrentFile": (("taskID",), (), "TorrentFile"),
    "/torrents/SSLParameters": (("hash",), (), "SSLParameters"),
    "/torrents/categories": ((), (), "CategoryMap"),
    "/torrents/count": ((), (), "CountText"),
    "/torrents/downloadLimit": (("hashes",), (), "LimitMap"),
    "/torrents/export": (("hash",), (), "TorrentFile"),
    "/torrents/files": (("hash",), ("indexes",), "TorrentFileInfoList"),
    "/torrents/info": ((), ("filter", "category", "tag", "sort", "reverse", "limit", "offset", "hashes", "private", "includeFiles", "includeTrackers"), "TorrentInfoList"),
    "/torrents/pieceAvailability": (("hash",), (), "IntegerList"),
    "/torrents/pieceHashes": (("hash",), (), "StringList"),
    "/torrents/pieceStates": (("hash",), (), "IntegerList"),
    "/torrents/properties": (("hash",), (), "TorrentProperties"),
    "/torrents/saveMetadata": (("source",), (), "TorrentFile"),
    "/torrents/tags": ((), (), "StringList"),
    "/torrents/trackers": (("hash",), (), "TorrentTrackerList"),
    "/torrents/uploadLimit": (("hashes",), (), "LimitMap"),
    "/torrents/webseeds": (("hash",), (), "TorrentWebSeedList"),
    "/transfer/downloadLimit": ((), (), "IntegerText"),
    "/transfer/info": ((), (), "TransferInfo"),
    "/transfer/speedLimitsMode": ((), (), "IntegerText"),
    "/transfer/uploadLimit": ((), (), "IntegerText"),
}

POST_ROUTES = {
    "/app/deleteAPIKey": ((), (), "EmptyText"),
    "/app/rotateAPIKey": ((), (), "APIKey"),
    "/app/sendTestEmail": ((), (), "EmptyText"),
    "/app/setCookies": (("cookies",), (), "EmptyText"),
    "/app/setPreferences": (("json",), (), "EmptyText"),
    "/app/shutdown": ((), (), "EmptyText"),
    "/clientdata/store": (("data",), (), "EmptyText"),
    "/rss/addFeed": (("url", "path"), ("refreshInterval",), "EmptyText"),
    "/rss/addFolder": (("path",), (), "EmptyText"),
    "/rss/markAsRead": (("itemPath",), ("articleId",), "EmptyText"),
    "/rss/moveItem": (("itemPath", "destPath"), (), "EmptyText"),
    "/rss/refreshItem": (("itemPath",), (), "EmptyText"),
    "/rss/removeItem": (("path",), (), "EmptyText"),
    "/rss/removeRule": (("ruleName",), (), "EmptyText"),
    "/rss/renameRule": (("ruleName", "newRuleName"), (), "EmptyText"),
    "/rss/setFeedRefreshInterval": (("path", "refreshInterval"), (), "EmptyText"),
    "/rss/setFeedURL": (("path", "url"), (), "EmptyText"),
    "/rss/setRule": (("ruleName", "ruleDef"), (), "EmptyText"),
    "/search/delete": (("id",), (), "EmptyText"),
    "/search/downloadTorrent": (("torrentUrl", "pluginName"), (), "EmptyText"),
    "/search/enablePlugin": (("names", "enable"), (), "EmptyText"),
    "/search/installPlugin": (("sources",), (), "EmptyText"),
    "/search/start": (("pattern", "category", "plugins"), (), "SearchStart"),
    "/search/stop": (("id",), (), "EmptyText"),
    "/search/uninstallPlugin": (("names",), (), "EmptyText"),
    "/search/updatePlugins": ((), (), "EmptyText"),
    "/torrentcreator/addTask": (("sourcePath",), ("private", "format", "optimizeAlignment", "paddedFileSizeLimit", "pieceSize", "torrentFilePath", "comment", "source", "trackers", "urlSeeds", "startSeeding"), "TorrentCreatorStart"),
    "/torrentcreator/deleteTask": (("taskID",), (), "EmptyText"),
    "/torrents/addPeers": (("hashes", "peers"), (), "DynamicObject"),
    "/torrents/addTags": (("hashes", "tags"), (), "EmptyText"),
    "/torrents/addTrackers": (("hash", "urls"), (), "EmptyText"),
    "/torrents/addWebSeeds": (("hash", "urls"), (), "EmptyText"),
    "/torrents/bottomPrio": (("hashes",), (), "EmptyText"),
    "/torrents/createCategory": (("category",), ("savePath", "downloadPath", "downloadPathEnabled"), "EmptyText"),
    "/torrents/createTags": (("tags",), (), "EmptyText"),
    "/torrents/decreasePrio": (("hashes",), (), "EmptyText"),
    "/torrents/delete": (("hashes", "deleteFiles"), (), "EmptyText"),
    "/torrents/deleteTags": (("tags",), (), "EmptyText"),
    "/torrents/editCategory": (("category", "savePath"), ("downloadPath", "downloadPathEnabled"), "EmptyText"),
    "/torrents/editTracker": (("hash", "url"), ("newUrl", "tier"), "EmptyText"),
    "/torrents/editWebSeed": (("hash", "origUrl", "newUrl"), (), "EmptyText"),
    "/torrents/fetchMetadata": (("source",), ("downloader",), "TorrentMetadata"),
    "/torrents/filePrio": (("hash", "id", "priority"), (), "EmptyText"),
    "/torrents/increasePrio": (("hashes",), (), "EmptyText"),
    "/torrents/reannounce": (("hashes",), ("urls",), "EmptyText"),
    "/torrents/recheck": (("hashes",), (), "EmptyText"),
    "/torrents/removeCategories": (("categories",), (), "EmptyText"),
    "/torrents/removeTags": (("hashes",), ("tags",), "EmptyText"),
    "/torrents/removeTrackers": (("hash", "urls"), (), "EmptyText"),
    "/torrents/removeWebSeeds": (("hash", "urls"), (), "EmptyText"),
    "/torrents/rename": (("hash", "name"), (), "EmptyText"),
    "/torrents/renameFile": (("hash", "oldPath", "newPath"), (), "EmptyText"),
    "/torrents/renameFolder": (("hash", "oldPath", "newPath"), (), "EmptyText"),
    "/torrents/setAutoManagement": (("hashes", "enable"), (), "EmptyText"),
    "/torrents/setCategory": (("hashes", "category"), (), "EmptyText"),
    "/torrents/setComment": (("hashes", "comment"), (), "EmptyText"),
    "/torrents/setDownloadLimit": (("hashes", "limit"), (), "EmptyText"),
    "/torrents/setDownloadPath": (("id", "path"), (), "EmptyText"),
    "/torrents/setForceStart": (("hashes", "value"), (), "EmptyText"),
    "/torrents/setLocation": (("hashes", "location"), (), "EmptyText"),
    "/torrents/setSSLParameters": (("hash", "ssl_certificate", "ssl_private_key", "ssl_dh_params"), (), "EmptyText"),
    "/torrents/setSavePath": (("id", "path"), (), "EmptyText"),
    "/torrents/setShareLimits": (("hashes", "ratioLimit", "seedingTimeLimit", "inactiveSeedingTimeLimit", "shareLimitAction"), (), "EmptyText"),
    "/torrents/setSuperSeeding": (("hashes", "value"), (), "EmptyText"),
    "/torrents/setTags": (("hashes", "tags"), (), "EmptyText"),
    "/torrents/setUploadLimit": (("hashes", "limit"), (), "EmptyText"),
    "/torrents/start": (("hashes",), (), "EmptyText"),
    "/torrents/stop": (("hashes",), (), "EmptyText"),
    "/torrents/toggleFirstLastPiecePrio": (("hashes",), (), "EmptyText"),
    "/torrents/toggleSequentialDownload": (("hashes",), (), "EmptyText"),
    "/torrents/topPrio": (("hashes",), (), "EmptyText"),
    "/transfer/banPeers": (("peers",), (), "EmptyText"),
    "/transfer/setDownloadLimit": (("limit",), (), "EmptyText"),
    "/transfer/setSpeedLimitsMode": (("mode",), (), "EmptyText"),
    "/transfer/setUploadLimit": (("limit",), (), "EmptyText"),
    "/transfer/toggleSpeedLimitsMode": ((), (), "EmptyText"),
}

MULTIPART_ROUTES = {
    "/torrents/add": (
        (),
        (
            "torrents", "urls", "savepath", "downloadPath", "useDownloadPath", "category", "tags",
            "skip_checking", "sequentialDownload", "firstLastPiecePrio", "forced", "addToTopOfQueue",
            "stopped", "rename", "upLimit", "dlLimit", "ratioLimit", "seedingTimeLimit",
            "inactiveSeedingTimeLimit", "shareLimitAction", "autoTMM", "stopCondition", "contentLayout",
            "filePriorities", "downloader", "ssl_certificate", "ssl_private_key", "ssl_dh_params",
        ),
        "EmptyText",
    ),
    "/torrents/parseMetadata": (("torrent",), (), "TorrentMetadataList"),
}

BOOLEAN_FIELDS = {
    "normal", "info", "warning", "critical", "withMetadata", "withData", "private", "enable",
    "addToTopOfQueue", "autoTMM", "firstLastPiecePrio", "forced", "sequentialDownload",
    "skip_checking", "stopped", "useDownloadPath", "deleteFiles", "downloadPathEnabled",
    "startSeeding", "optimizeAlignment", "value", "includeFiles", "includeTrackers", "reverse",
}
INTEGER_FIELDS = {
    "last_known_id", "rid", "refreshInterval", "limit", "offset", "mode", "pieceSize",
    "paddedFileSizeLimit", "upLimit", "dlLimit", "seedingTimeLimit", "inactiveSeedingTimeLimit",
    "priority", "tier",
}
NUMBER_FIELDS = {"ratioLimit"}
JSON_STRING_FIELDS = {"json", "cookies", "keys", "data", "ruleDef"}

ENUMS = {
    ("/app/getDirectoryContent", "mode"): ["all", "dirs", "files"],
    ("/torrentcreator/addTask", "format"): ["v1", "v2", "hybrid"],
    ("/torrents/add", "contentLayout"): ["Original", "Subfolder", "NoSubfolder"],
    ("/torrents/add", "shareLimitAction"): ["Default", "Stop", "Remove", "RemoveWithContent", "EnableSuperSeeding"],
    ("/torrents/add", "stopCondition"): ["None", "MetadataReceived", "FilesChecked"],
    ("/torrents/filePrio", "priority"): [0, 1, 6, 7],
    ("/torrents/info", "filter"): ["all", "downloading", "seeding", "completed", "stopped", "active", "inactive", "running", "stalled", "stalled_uploading", "stalled_downloading", "errored"],
    ("/torrents/setShareLimits", "shareLimitAction"): ["Default", "Stop", "Remove", "RemoveWithContent", "EnableSuperSeeding"],
}

OPERATION_ID_OVERRIDES = {
    "/app/webapiVersion": "get_webapi_version",
    "/torrents/info": "get_torrents",
    "/torrents/count": "get_torrent_count",
    "/torrents/properties": "get_torrent_properties",
    "/torrents/trackers": "get_torrent_trackers",
    "/torrents/webseeds": "get_torrent_webseeds",
    "/torrents/files": "get_torrent_files",
    "/torrents/pieceHashes": "get_torrent_piece_hashes",
    "/torrents/pieceStates": "get_torrent_piece_states",
    "/torrents/pieceAvailability": "get_torrent_piece_availability",
    "/torrents/SSLParameters": "get_torrent_ssl_parameters",
    "/torrents/export": "get_torrent_export",
    "/torrents/saveMetadata": "get_torrent_metadata_file",
}

SUMMARY_OVERRIDES = {
    "/app/getDirectoryContent": "List server directory contents",
    "/app/shutdown": "Shut down qBittorrent",
    "/torrents/add": "Add a torrent from a URL or uploaded torrent file",
    "/torrents/fetchMetadata": "Fetch torrent metadata asynchronously",
    "/torrents/parseMetadata": "Parse an uploaded torrent file",
    "/torrents/saveMetadata": "Download fetched torrent metadata",
}


def snake(name: str) -> str:
    return re.sub(r"(?<!^)(?=[A-Z])", "_", name).lower()


def operation_id(method: str, path: str) -> str:
    if path in OPERATION_ID_OVERRIDES:
        return OPERATION_ID_OVERRIDES[path]
    scope, action = path.strip("/").split("/")
    scope_name = {"clientdata": "client_data", "torrentcreator": "torrent_creator"}.get(scope, scope)
    return f"{method}_{scope_name}_{snake(action)}"


def summary(method: str, path: str) -> str:
    if path in SUMMARY_OVERRIDES:
        return SUMMARY_OVERRIDES[path]
    scope, action = path.strip("/").split("/")
    words = snake(action).replace("_", " ")
    verb = "Get" if method == "get" else "Run"
    return f"{verb} {scope} {words}"


def field_schema(path: str, name: str) -> dict:
    schema: dict = {"type": "string"}
    if name == "torrents" or (path == "/torrents/parseMetadata" and name == "torrent"):
        schema = {"type": "string", "format": "binary"}
    elif path == "/app/getDirectoryContent" and name == "mode":
        schema = {"type": "string"}
    elif name in BOOLEAN_FIELDS:
        schema = {"type": "boolean"}
    elif name in INTEGER_FIELDS or (path.startswith("/search/") and name == "id"):
        schema = {"type": "integer", "format": "int64"}
    elif name in NUMBER_FIELDS:
        schema = {"type": "number", "format": "double"}
    if enum := ENUMS.get((path, name)):
        schema["enum"] = enum
    if name in JSON_STRING_FIELDS:
        schema["description"] = "JSON encoded as a string form/query value. Use JSON.stringify before calling."
    if name in {"hashes", "id"} and not (path.startswith("/search/") and name == "id"):
        schema.setdefault("description", "Pipe-delimited identifiers; `all` is accepted where supported by qBittorrent.")
        schema["minLength"] = 1
    elif name == "hash":
        schema["minLength"] = 1
    return schema


def response_ref(key: str) -> dict:
    if key == "EmptyText":
        return {"description": "Success", "content": {"text/plain": {"schema": {"type": "string"}}}}
    if key == "Text":
        return {"description": "Success", "content": {"text/plain": {"schema": {"type": "string"}}}}
    if key == "IntegerText":
        return {
            "description": "Success",
            "content": {
                "text/plain": {
                    "schema": {
                        "type": "string",
                        "description": "Decimal integer encoded as text. Parse the response text as a number.",
                    }
                }
            },
        }
    if key == "CountText":
        return {
            "description": "Success",
            "content": {
                "text/plain": {
                    "schema": {
                        "type": "string",
                        "description": "Torrent count encoded as decimal text. Parse the response text as a number.",
                    }
                }
            },
        }
    if key == "TorrentFile":
        return {"description": "Torrent metainfo file", "content": {"application/x-bittorrent": {"schema": {"type": "string", "format": "binary"}}}}
    return {"description": "Success", "content": {"application/json": {"schema": {"$ref": f"#/components/schemas/{key}"}}}}


def operation(method: str, path: str, required: tuple[str, ...], optional: tuple[str, ...], response: str, multipart: bool = False) -> dict:
    scope = path.strip("/").split("/", 1)[0]
    value = {
        "tags": [scope],
        "operationId": operation_id(method, path),
        "summary": summary(method, path),
        "responses": {"200": response_ref(response)},
    }
    names = required + optional
    if method == "get":
        value["parameters"] = [
            {
                "name": name,
                "in": "query",
                "required": name in required,
                "schema": field_schema(path, name),
            }
            for name in names
        ]
    elif names:
        properties = {name: field_schema(path, name) for name in names}
        body_schema = {"type": "object", "properties": properties, "additionalProperties": False}
        if required:
            body_schema["required"] = list(required)
        if path == "/torrents/add":
            body_schema["anyOf"] = [{"required": ["urls"]}, {"required": ["torrents"]}]
        media_type = "multipart/form-data" if multipart else "application/x-www-form-urlencoded"
        media: dict = {"schema": body_schema}
        if multipart:
            binary_names = [name for name, schema in properties.items() if schema.get("format") == "binary"]
            if binary_names:
                media["encoding"] = {
                    name: {"contentType": "application/x-bittorrent"} for name in binary_names
                }
        value["requestBody"] = {"required": bool(required) or multipart, "content": {media_type: media}}
    if path == "/torrents/fetchMetadata":
        value["responses"]["202"] = response_ref(response)
    return value


def obj(properties: dict, required: tuple[str, ...] = (), additional: bool = True) -> dict:
    result = {"type": "object", "properties": properties, "additionalProperties": additional}
    if required:
        result["required"] = list(required)
    return result


def array(items: dict) -> dict:
    return {"type": "array", "items": items}


def components() -> dict:
    string = {"type": "string"}
    integer = {"type": "integer", "format": "int64"}
    number = {"type": "number", "format": "double"}
    boolean = {"type": "boolean"}
    schemas = {
        "DynamicObject": {"type": "object", "additionalProperties": True},
        "Text": string,
        "Integer": integer,
        "StringList": array(string),
        "IntegerList": array(integer),
        "BuildInfo": obj({"qt": string, "libtorrent": string, "boost": string, "openssl": string, "zlib": string, "bitness": integer, "platform": string}),
        "ProcessInfo": obj({"launch_time": integer}, ("launch_time",)),
        "Cookie": obj({"name": string, "domain": string, "path": string, "value": string, "expirationDate": integer}),
        "DirectoryEntry": obj({"name": string, "type": {"type": "string", "enum": ["dir", "file"]}, "size": integer, "creation_date": integer, "last_access_date": integer, "last_modification_date": integer}),
        "NetworkInterface": obj({"name": string, "value": string}, ("name", "value")),
        "LogEntry": obj({"id": integer, "timestamp": integer, "type": integer, "message": string}),
        "PeerLogEntry": obj({"id": integer, "timestamp": integer, "ip": string, "blocked": boolean, "reason": string}),
        "TransferInfo": obj({"dl_info_speed": integer, "dl_info_data": integer, "up_info_speed": integer, "up_info_data": integer, "dl_rate_limit": integer, "up_rate_limit": integer, "dht_nodes": integer, "connection_status": string, "queueing": boolean, "use_alt_speed_limits": boolean, "refresh_interval": integer}),
        "TorrentInfo": obj({"hash": string, "name": string, "size": integer, "progress": number, "dlspeed": integer, "upspeed": integer, "priority": integer, "num_seeds": integer, "num_leechs": integer, "ratio": number, "eta": integer, "state": string, "seq_dl": boolean, "f_l_piece_prio": boolean, "category": string, "tags": string, "save_path": string, "download_path": string, "added_on": integer, "completion_on": integer, "tracker": string, "trackers_count": integer, "files_count": integer, "private": boolean}),
        "TorrentProperties": obj({"time_elapsed": integer, "seeding_time": integer, "eta": integer, "nb_connections": integer, "nb_connections_limit": integer, "total_downloaded": integer, "total_downloaded_session": integer, "total_uploaded": integer, "total_uploaded_session": integer, "dl_speed": integer, "dl_speed_avg": integer, "up_speed": integer, "up_speed_avg": integer, "dl_limit": integer, "up_limit": integer, "total_wasted": integer, "seeds": integer, "seeds_total": integer, "peers": integer, "peers_total": integer, "share_ratio": number, "popularity": number, "reannounce": integer, "total_size": integer, "pieces_num": integer, "piece_size": integer, "pieces_have": integer, "created_by": string, "last_seen": integer, "addition_date": integer, "completion_date": integer, "creation_date": integer, "save_path": string, "download_path": string, "comment": string, "private": boolean, "has_metadata": boolean}),
        "TorrentTracker": obj({"url": string, "name": string, "status": integer, "tier": integer, "msg": string, "num_peers": integer, "num_seeds": integer, "num_leeches": integer, "num_downloaded": integer, "next_announce": integer, "min_announce": integer}),
        "TorrentWebSeed": obj({"url": string}, ("url",)),
        "TorrentFileInfo": obj({"index": integer, "name": string, "size": integer, "progress": number, "priority": integer, "is_seed": boolean, "piece_range": array(integer), "availability": number}),
        "Category": obj({"name": string, "savePath": string, "downloadPath": string, "downloadPathEnabled": boolean}),
        "SearchPlugin": obj({"name": string, "fullName": string, "url": string, "version": string, "enabled": boolean, "supportedCategories": array(obj({"id": string, "name": string}))}),
        "SearchStatus": obj({"id": integer, "status": string, "total": integer}),
        "SearchResult": obj({"descrLink": string, "fileName": string, "fileSize": integer, "fileUrl": string, "nbLeechers": integer, "nbSeeders": integer, "siteUrl": string, "pubDate": integer}),
        "TorrentCreatorTask": obj({"taskID": string, "sourcePath": string, "pieceSize": integer, "private": boolean, "format": string, "status": string, "progress": number, "timeAdded": string, "timeStarted": string, "timeFinished": string, "errorMessage": string}),
        "SSLParameters": obj({"ssl_certificate": string, "ssl_private_key": string, "ssl_dh_params": string}),
        "APIKey": obj({"apiKey": string}, ("apiKey",)),
        "SearchStart": obj({"id": integer}, ("id",)),
        "TorrentCreatorStart": obj({"taskID": string}, ("taskID",)),
        "TorrentMetadata": obj({"hash": string, "name": string, "files": array(obj({"name": string, "size": integer})), "trackers": array(string)}),
        "LimitMap": {"type": "object", "additionalProperties": integer},
        "CategoryMap": {"type": "object", "additionalProperties": {"$ref": "#/components/schemas/Category"}},
    }
    schemas.update({
        "CookieList": array({"$ref": "#/components/schemas/Cookie"}),
        "DirectoryContents": array({"oneOf": [string, {"$ref": "#/components/schemas/DirectoryEntry"}]}),
        "NetworkInterfaceList": array({"$ref": "#/components/schemas/NetworkInterface"}),
        "LogEntryList": array({"$ref": "#/components/schemas/LogEntry"}),
        "PeerLogEntryList": array({"$ref": "#/components/schemas/PeerLogEntry"}),
        "TorrentInfoList": array({"$ref": "#/components/schemas/TorrentInfo"}),
        "TorrentTrackerList": array({"$ref": "#/components/schemas/TorrentTracker"}),
        "TorrentWebSeedList": array({"$ref": "#/components/schemas/TorrentWebSeed"}),
        "TorrentFileInfoList": array({"$ref": "#/components/schemas/TorrentFileInfo"}),
        "SearchPluginList": array({"$ref": "#/components/schemas/SearchPlugin"}),
        "SearchStatusList": array({"$ref": "#/components/schemas/SearchStatus"}),
        "SearchResults": obj({"status": string, "total": integer, "results": array({"$ref": "#/components/schemas/SearchResult"})}),
        "TorrentCreatorTaskList": array({"$ref": "#/components/schemas/TorrentCreatorTask"}),
        "TorrentMetadataList": array({"$ref": "#/components/schemas/TorrentMetadata"}),
    })
    return {
        "securitySchemes": {
            "SID": {
                "type": "apiKey",
                "in": "cookie",
                "name": "SID",
                "description": (
                    "Yarr's qBittorrent transport logs in and manages the upstream SID cookie. "
                    "qBittorrent API-key bearer authentication is an upstream alternative but is not used by this transport."
                ),
            },
        },
        "schemas": schemas,
    }


def build() -> tuple[dict, dict]:
    paths: dict[str, dict] = {}
    inventory = []
    for method, routes, multipart in (("get", GET_ROUTES, False), ("post", POST_ROUTES, False), ("post", MULTIPART_ROUTES, True)):
        for path, (required, optional, response) in routes.items():
            paths.setdefault(path, {})[method] = operation(method, path, required, optional, response, multipart)
            scope = path.strip("/").split("/", 1)[0]
            inventory.append({
                "path": path,
                "method": method,
                "operationId": operation_id(method, path),
                "source": f"src/webui/api/{'torrentcreator' if scope == 'torrentcreator' else scope}controller.h",
                "action": path.rsplit("/", 1)[1],
                "status": "specified",
            })

    spec = {
        "openapi": "3.0.3",
        "info": {
            "title": "qBittorrent WebUI API",
            "version": "5.2.3-webapi-2.15.1",
            "description": "Local maintained contract for the qBittorrent 5.2.3 WebUI API, pinned to official source and the official 5.0+ API wiki.",
            "x-upstream-commit": UPSTREAM["commit"],
        },
        "servers": [{"url": "/api/v2"}],
        "security": [{"SID": []}],
        "tags": [{"name": name} for name in ("app", "clientdata", "log", "rss", "search", "sync", "torrentcreator", "torrents", "transfer")],
        "paths": dict(sorted(paths.items())),
        "components": components(),
    }
    coverage = {
        "schemaVersion": 1,
        "service": "qbittorrent",
        "apiVersion": "2.15.1",
        "upstream": UPSTREAM,
        "inventory": sorted(inventory, key=lambda row: row["path"]),
        "exclusions": [
            {
                "path": "/auth/login",
                "method": "post",
                "source": "src/webui/api/authcontroller.h",
                "action": "login",
                "status": "excluded",
                "reason": "Yarr transport owns qBittorrent authentication and session establishment; exposing credentials as an operation would duplicate and bypass transport auth.",
            },
            {
                "path": "/auth/logout",
                "method": "post",
                "source": "src/webui/api/authcontroller.h",
                "action": "logout",
                "status": "excluded",
                "reason": "Yarr transport owns the authenticated qBittorrent session lifecycle; callers must not invalidate the pooled transport session.",
            },
        ],
        "nonApiExclusions": [
            {
                "surface": "src/webui/www/**",
                "reason": "Browser HTML, JavaScript, static resources, and alternate WebUI assets are UI implementation, not /api/v2 controller routes.",
            }
        ],
    }
    return spec, coverage


def main() -> None:
    spec, coverage = build()
    SPEC_PATH.write_text(json.dumps(spec, indent=2, sort_keys=False) + "\n", encoding="utf-8")
    COVERAGE_PATH.write_text(json.dumps(coverage, indent=2, sort_keys=False) + "\n", encoding="utf-8")
    print(f"wrote {SPEC_PATH.relative_to(ROOT)} ({sum(len(item) for item in spec['paths'].values())} operations)")
    print(f"wrote {COVERAGE_PATH.relative_to(ROOT)} ({len(coverage['inventory'])} mapped, {len(coverage['exclusions'])} excluded)")


if __name__ == "__main__":
    main()
