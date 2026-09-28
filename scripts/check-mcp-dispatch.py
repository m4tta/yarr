#!/usr/bin/env python3
"""Exercise real stdio dispatch against an owned loopback fake.

Uses no production config or media. The client never supplies approval answers;
valid authorized requests dispatch, while invalid input never reaches upstream.
"""

import argparse
import asyncio
import json
import os
from pathlib import Path
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs


class Upstream(BaseHTTPRequestHandler):
    writes = []

    def log_message(self, *_args):
        pass

    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("Content-Length", "0")))
        if self.path == "/api/v2/auth/login":
            self.send_response(200)
            self.send_header("Set-Cookie", "SID=synthetic-test-session; Path=/")
            self.end_headers()
            self.wfile.write(b"Ok.")
        elif self.path == "/api/v2/torrents/delete":
            self.writes.append(parse_qs(body.decode()))
            self.send_response(200)
            self.end_headers()
        else:
            self.send_error(404)


async def run_case(binary, endpoint, case, arguments, capabilities, allow,
                   tool_mode="codemode"):
    with tempfile.TemporaryDirectory(prefix="yarr-dispatch-") as temporary:
        home = Path(temporary)
        config = home / "config.toml"
        config.write_text("", encoding="utf-8")
        env = {
            key: os.environ[key]
            for key in ("PATH", "SYSTEMROOT", "WINDIR")
            if key in os.environ
        }
        env.update({
            "HOME": temporary,
            "YARR_HOME": temporary,
            "YARR_CONFIG": str(config),
            "YARR_SERVICES": "qbit_test",
            "YARR_QBIT_TEST_KIND": "qbittorrent",
            "YARR_QBIT_TEST_URL": endpoint,
            "YARR_QBIT_TEST_USERNAME": "synthetic",
            "YARR_QBIT_TEST_PASSWORD": "synthetic",
            "YARR_MCP_TOOL_MODE": tool_mode,
            "RUST_LOG": "warn",
        })
        process = await asyncio.create_subprocess_exec(
            str(binary), "mcp", cwd=temporary, env=env,
            stdin=asyncio.subprocess.PIPE, stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
        stderr = asyncio.create_task(process.stderr.read())
        before = len(Upstream.writes)

        async def send(message):
            process.stdin.write((json.dumps(message) + "\n").encode())
            await process.stdin.drain()

        async def request(identifier, method, params):
            await send({"jsonrpc": "2.0", "id": identifier,
                        "method": method, "params": params})
            while True:
                line = await asyncio.wait_for(process.stdout.readline(), 15)
                assert line, f"{case}: server exited before response"
                message = json.loads(line)
                if message.get("method") == "elicitation/create":
                    raise AssertionError(f"{case}: unexpected confirmation request")
                elif message.get("id") == identifier and "method" not in message:
                    assert "error" not in message, message
                    return message["result"]

        try:
            await request(1, "initialize", {
                "protocolVersion": "2025-11-25", "capabilities": capabilities,
                "clientInfo": {"name": "yarr-dispatch-regression", "version": "1"},
            })
            await send({"jsonrpc": "2.0", "method": "notifications/initialized"})
            result = await request(2, "tools/call", {
                "name": "yarr" if tool_mode == "codemode" else "qbit_test",
                "arguments": arguments,
            })
            writes = Upstream.writes[before:]
            assert bool(writes) == allow, (case, writes, result)
            assert bool(result.get("isError")) != allow, (case, result)
            if allow:
                assert len(writes) == 1, writes
                assert writes[0] == {
                    "hashes": ["0123456789012345678901234567890123456789"],
                    "deleteFiles": ["false"],
                }, writes
            return {"case": case, "prompts": 0, "upstreamWrites": len(writes)}
        finally:
            process.stdin.close()
            try:
                await asyncio.wait_for(process.wait(), 3)
            except asyncio.TimeoutError:
                process.kill()
                await process.wait()
            await stderr


async def main(binary):
    form = {"elicitation": {"form": {}}}
    body = {"hashes": "0123456789012345678901234567890123456789", "deleteFiles": False}

    def generated(payload):
        return {"code": "async () => await qbit_test.post_torrents_delete({body:"
                + json.dumps(payload) + "})"}

    cases = [
        ("generated-without-form", generated(body), {}, True),
        ("generated-with-form", generated(body), form, True),
        ("curated-without-form", {"code": "async () => await qbit_test.download_remove({"
         "id:'0123456789012345678901234567890123456789',delete_files:false})"}, {}, True),
        ("flat-generated", {"action": "op", "op": "post_torrents_delete",
                            "args": {"body": body}}, {}, True, "flat"),
        ("invalid-boolean", generated({**body, "deleteFiles": "false"}), {}, False),
        ("missing-selector", generated({"deleteFiles": False}), form, False),
    ]
    server = ThreadingHTTPServer(("127.0.0.1", 0), Upstream)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        endpoint = f"http://127.0.0.1:{server.server_port}"
        results = [await run_case(binary, endpoint, *case) for case in cases]
        print(json.dumps({"passed": len(results), "cases": results}, indent=2))
    finally:
        server.shutdown()
        server.server_close()
        thread.join()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    args = parser.parse_args()
    asyncio.run(main(args.binary.resolve(strict=True)))
