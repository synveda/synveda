"""ADPT-4 public HTTP client; the gateway owns policy and application state."""
from __future__ import annotations

import asyncio
import json
import re
import secrets
from collections.abc import Awaitable, Callable, Mapping
from dataclasses import dataclass
from importlib.resources import files
from typing import Any, Generic, TypeVar
from urllib.parse import quote, urlencode, urlsplit

import httpx

T = TypeVar("T")
TokenProvider = Callable[[bool], Awaitable[str]]
CONTRACT = json.loads(files("synveda").joinpath("contract.json").read_text())


@dataclass(frozen=True)
class ApiResponse(Generic[T]):
    data: T
    status: int
    trace_id: str
    retry_after: str | None = None


class ApiError(Exception):
    def __init__(self, status: int, detail: dict[str, Any], trace_id: str, retry_after: str | None = None):
        self.status, self.detail, self.trace_id, self.retry_after = status, detail, trace_id, retry_after
        super().__init__(f"Synveda request failed ({status}, {detail['kind']})")


class TransportError(Exception):
    def __init__(self, kind: str):
        self.kind = kind
        super().__init__(f"Synveda request failed ({kind})")


class Client:
    """Bounded async client. Use generated operations for typed bodies/results.

    Pagination and task lifetime remain explicit application decisions.
    Cancellation propagates as asyncio.CancelledError.
    """

    def __init__(self, base_url: str, token_provider: TokenProvider, *, timeout: float = 30,
                 max_response_bytes: int = 8 * 1024 * 1024):
        url = urlsplit(base_url)
        if url.scheme not in ("https", "http") or not url.hostname or url.username or url.password or url.query or url.fragment:
            raise ValueError("Use an HTTP(S) gateway URL without credentials, query or fragment")
        if not 0 < timeout <= 120 or type(max_response_bytes) is not int or not 0 < max_response_bytes <= 64 * 1024 * 1024:
            raise ValueError("Timeout or response limit is outside supported bounds")
        self._base = base_url.rstrip("/")
        self._token = token_provider
        self._timeout = timeout
        self._max_bytes = max_response_bytes
        self._http = httpx.AsyncClient(timeout=timeout, follow_redirects=False,
                                      limits=httpx.Limits(max_connections=10, max_keepalive_connections=5))

    async def __aenter__(self) -> Client:
        return self

    async def __aexit__(self, *_: Any) -> None:
        await self.aclose()

    async def aclose(self) -> None:
        await self._http.aclose()

    async def request(self, operation: str, *, body: Any = None, path: Mapping[str, str] | None = None,
                      query: Mapping[str, str | int | bool | None] | None = None,
                      idempotency_key: str | None = None, traceparent: str | None = None) -> ApiResponse[Any]:
        route = CONTRACT["operations"].get(operation)
        if route is None:
            raise ValueError("Operation is outside this SDK contract slice")
        url = self._url(route, path or {}, query or {})
        payload = None if body is None else json.dumps(body, ensure_ascii=False, separators=(",", ":"), allow_nan=False).encode()
        if payload is not None and len(payload) > self._max_bytes:
            raise ValueError("Request body too large")
        if route["body"] and payload is None:
            raise ValueError("Request body is required")
        if not route["accepts_body"] and payload is not None:
            raise ValueError("This operation takes no body")
        if not route["idempotent"] and idempotency_key is not None:
            raise ValueError("This operation has no request idempotency contract")
        if route["idempotent"] and not _header(idempotency_key):
            raise ValueError("Idempotency key is required (1–200 ASCII characters)")
        parent = traceparent or f"00-{secrets.token_hex(16)}-{secrets.token_hex(8)}-01"
        if not re.fullmatch(r"00-[0-9a-f]{32}-[0-9a-f]{16}-0[01]", parent) or set(parent[3:35]) == {"0"} or set(parent[36:52]) == {"0"}:
            raise ValueError("Invalid W3C traceparent")
        try:
            async with asyncio.timeout(self._timeout):
                return await self._send(route, url, payload, idempotency_key, parent)
        except (TimeoutError, httpx.TimeoutException):
            raise TransportError("timeout") from None
        except httpx.HTTPError:
            # HTTPX exceptions can retain request credentials; do not propagate them.
            raise TransportError("transport") from None

    async def _send(self, route: dict[str, Any], url: str, body: bytes | None,
                    key: str | None, parent: str) -> ApiResponse[Any]:
        token = await self._bearer(False)
        for attempt in range(2):
            if not _header(token, 16_384):
                raise ValueError("Bearer provider returned an invalid token")
            headers = {"authorization": f"Bearer {token}", "traceparent": parent,
                       "x-synveda-client": "synveda-python/0.1.0", "accept": "application/json"}
            if body is not None:
                headers["content-type"] = "application/json"
            if route["idempotent"]:
                headers["idempotency-key"] = key
            async with self._http.stream(route["method"], url, content=body, headers=headers) as response:
                data = await self._read(response)
                status = response.status_code
                retry_after = response.headers.get("retry-after")
            if 200 <= status < 300:
                return ApiResponse(data, status, parent[3:35], retry_after)
            if status == 401 and attempt == 0 and (route["method"] == "GET" or route["idempotent"]):
                refreshed = await self._bearer(True)
                if refreshed != token:
                    token = refreshed
                    continue
            detail = data if isinstance(data, dict) and isinstance(data.get("kind"), str) else {"kind": "invalid_response"}
            raise ApiError(status, detail, parent[3:35], retry_after)
        raise TransportError("transport")

    async def _bearer(self, refresh: bool) -> str:
        try:
            return await self._token(refresh)
        except Exception:
            # Provider errors can embed credentials. Cancellation is a BaseException
            # and continues to propagate to the application's task owner.
            raise TransportError("transport") from None

    async def _read(self, response: httpx.Response) -> Any:
        if response.status_code == 204:
            return None
        content = bytearray()
        async for chunk in response.aiter_bytes():
            if len(content) + len(chunk) > self._max_bytes:
                raise TransportError("response_too_large")
            content.extend(chunk)
        try:
            return json.loads(content)
        except (ValueError, UnicodeError):
            raise TransportError("invalid_response") from None

    def _url(self, route: dict[str, Any], path: Mapping[str, str], query: Mapping[str, Any]) -> str:
        for location, supplied in (("path", path), ("query", query)):
            parameters = [p for p in route["parameters"] if p["location"] == location]
            if set(supplied) - {p["name"] for p in parameters}:
                raise ValueError(f"Unknown {location} parameter")
            if any(p["required"] and supplied.get(p["name"]) is None for p in parameters):
                raise ValueError(f"Missing {location} parameter")
        suffix = route["path"]
        for name, value in path.items():
            if not isinstance(value, str) or not value or value in (".", ".."):
                raise ValueError("Invalid path parameter")
            suffix = suffix.replace("{" + name + "}", quote(value, safe=""))
        encoded = {}
        for name, value in query.items():
            if value is not None:
                if not isinstance(value, (str, int, bool)):
                    raise ValueError("Invalid query value")
                encoded[name] = str(value).lower() if isinstance(value, bool) else str(value)
        # Match WHATWG URLSearchParams used by the TypeScript client.
        query_string = urlencode(encoded, safe="*").replace("~", "%7E")
        return self._base + suffix + ("?" + query_string if encoded else "")


def _header(value: str | None, maximum: int = 200) -> bool:
    return isinstance(value, str) and 0 < len(value) <= maximum and re.fullmatch(r"[\x21-\x7e]+", value) is not None
