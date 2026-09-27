"""Small deterministic, tracker-free torrent fixtures using only the stdlib."""

from __future__ import annotations

import hashlib
from pathlib import Path


def bencode(value: object) -> bytes:
    if isinstance(value, int):
        return b"i" + str(value).encode() + b"e"
    if isinstance(value, bytes):
        return str(len(value)).encode() + b":" + value
    if isinstance(value, str):
        return bencode(value.encode())
    if isinstance(value, list):
        return b"l" + b"".join(bencode(item) for item in value) + b"e"
    if isinstance(value, dict):
        pairs = []
        for key in sorted(value, key=lambda item: str(item).encode()):
            pairs.append(bencode(key))
            pairs.append(bencode(value[key]))
        return b"d" + b"".join(pairs) + b"e"
    raise TypeError(f"cannot bencode {type(value).__name__}")


def create_fixture(directory: Path, nonce: str) -> tuple[Path, Path, str]:
    """Create a payload and tracker-free torrent; return paths and info hash."""
    directory.mkdir(parents=True, exist_ok=True)
    name = f"yarr-qbit-lab-{nonce}.bin"
    payload = directory / name
    torrent = directory / f"{name}.torrent"
    data = (b"yarr-qbit-lab\0" + nonce.encode() + b"\n") * 8192
    payload.write_bytes(data)
    piece_length = 16 * 1024
    pieces = b"".join(
        hashlib.sha1(data[offset : offset + piece_length]).digest()
        for offset in range(0, len(data), piece_length)
    )
    info = {
        b"length": len(data),
        b"name": name,
        b"piece length": piece_length,
        b"pieces": pieces,
        b"private": 1,
    }
    torrent.write_bytes(bencode({b"created by": b"yarr qbit lab", b"info": info}))
    return payload, torrent, hashlib.sha1(bencode(info)).hexdigest()
