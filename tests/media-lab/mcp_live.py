"""Bounded stdio MCP checks against one isolated media-lab service."""
import json
import selectors
import subprocess
import time
def _send(process, message):
    process.stdin.write(json.dumps(message, separators=(",", ":")) + "\n")
    process.stdin.flush()

def _receive(process, request_id, timeout=20):
    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ)
    deadline = time.monotonic() + timeout
    try:
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not selector.select(remaining):
                raise TimeoutError(f"MCP request {request_id} timed out")
            line = process.stdout.readline()
            if not line:
                raise RuntimeError(f"MCP process exited during request {request_id}")
            try:
                message = json.loads(line)
            except json.JSONDecodeError as error:
                raise RuntimeError("MCP process returned invalid JSON") from error
            if message.get("id") == request_id:
                return message
    finally:
        selector.close()

def _request(process, request_id, method, params=None):
    message = {"jsonrpc": "2.0", "id": request_id, "method": method}
    if params is not None:
        message["params"] = params
    _send(process, message)
    return _receive(process, request_id)

def _result(response):
    if "error" in response:
        raise RuntimeError(response["error"].get("message", "MCP JSON-RPC error"))
    return response["result"]

def _tool(process, request_id, service, arguments):
    return _request(process, request_id, "tools/call", {
        "name": service, "arguments": arguments,
    })

def _text(result):
    value = result["content"][0]["text"]
    try:
        return json.loads(value)
    except json.JSONDecodeError:
        return value

def check_mcp(lab, service):
    """Verify flat-tool success, validation, and fail-closed elicitation."""
    env = dict(lab.env, YARR_MCP_TOOL_MODE="flat")
    process = subprocess.Popen([lab.binary, "mcp"], env=env,
        stdin=subprocess.PIPE, stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL, text=True, bufsize=1)
    try:
        initialized = _result(_request(process, 1, "initialize", {
            "protocolVersion": "2025-11-25", "capabilities": {},
            "clientInfo": {"name": "yarr-media-lab", "version": "1"},
        }))
        if initialized.get("serverInfo", {}).get("name") != "yarr":
            raise AssertionError("Unexpected MCP server identity")
        _send(process, {"jsonrpc": "2.0", "method": "notifications/initialized"})
        read = _result(_tool(process, 2, service, {
            "action": "op", "op": "get_qualityprofile", "args": {},
        }))
        if read.get("isError") or not isinstance(_text(read), list):
            raise AssertionError("MCP generated read did not return a profile list")
        invalid = _tool(process, 3, service, {
            "action": "op", "op": "get_qualityprofile_by_id",
            "args": {"id": "not-an-integer"},
        })
        if "error" not in invalid and invalid.get("result", {}).get("isError") is not True:
            raise AssertionError("MCP invalid parameter was not rejected")
        if "schema validation" not in json.dumps(invalid):
            raise AssertionError("MCP failed for a reason other than schema validation")
        declined = _result(_tool(process, 4, service, {
            "action": "api_post", "path": "/api/v3/system/restart", "body": {},
        }))
        declined_body = _text(declined)
        if not isinstance(declined_body, dict) or declined_body.get("declined") is not True:
            raise AssertionError("MCP high-impact call did not fail closed")
        return {"transport": "stdio", "tool_mode": "flat", "tool": service,
                "verified": ["generated read", "invalid parameter rejection",
                             "high-impact call declined without elicitation"]}
    finally:
        if process.stdin:
            process.stdin.close()
        try:
            process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            process.terminate()
            try:
                process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=3)
