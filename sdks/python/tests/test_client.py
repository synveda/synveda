import asyncio
import json
import unittest
from pathlib import Path

import httpx
from synveda import ApiError, Client, TransportError

PARENT = "00-11111111111111111111111111111111-2222222222222222-01"


class ClientTests(unittest.IsolatedAsyncioTestCase):
    async def client(self, handler, token=None, **options):
        async def default_token(_refresh):
            return "synthetic-token"
        client = Client("http://fixture.test", token or default_token, **options)
        await client._http.aclose()
        client._http = httpx.AsyncClient(transport=httpx.MockTransport(handler), follow_redirects=False)
        self.addAsyncCleanup(client.aclose)
        return client

    async def test_shared_wire_fixtures(self):
        fixtures = json.loads((Path(__file__).parents[2] / "fixtures/wire.json").read_text())
        observed = []
        def handler(request):
            fixture = fixtures[len(observed)]
            observed.append(request)
            return httpx.Response(200, json=fixture["response"])
        client = await self.client(handler)
        for fixture in fixtures:
            options = dict(fixture["options"])
            if "idempotencyKey" in options:
                options["idempotency_key"] = options.pop("idempotencyKey")
            result = await client.request(fixture["operation"], **options, traceparent=PARENT)
            self.assertEqual(result.data, fixture["response"])
            self.assertEqual(result.trace_id, PARENT[3:35])
            request = observed[-1]
            self.assertEqual(request.method, fixture["method"])
            self.assertEqual(request.url.raw_path.decode(), fixture["url"])
            self.assertEqual(json.loads(request.content) if request.content else None, fixture["body"])
            self.assertEqual(request.headers["authorization"], "Bearer synthetic-token")
            self.assertEqual(request.headers.get("idempotency-key"), options.get("idempotency_key"))
            self.assertEqual(request.headers["traceparent"], PARENT)

    async def test_bounded_safe_refresh(self):
        refreshes, calls = [], []
        async def token(refresh):
            refreshes.append(refresh)
            return "fresh" if refresh else "expired"
        def handler(request):
            calls.append(request)
            return httpx.Response(200 if request.headers["authorization"] == "Bearer fresh" else 401, json={"kind": "unauthenticated"})
        client = await self.client(handler, token)
        await client.request("get_me")
        self.assertEqual(refreshes, [False, True])
        with self.assertRaises(ApiError):
            await client.request("append_session_events", path={"session_id": "s1"}, body={"events": []})
        self.assertEqual(refreshes, [False, True, False])
        self.assertEqual(len(calls), 3)

    async def test_rate_limit_is_not_silently_retried(self):
        calls = []
        def handler(request):
            calls.append(request)
            return httpx.Response(429, headers={"retry-after": "7"}, json={"kind": "rate_limited"})
        client = await self.client(handler)
        with self.assertRaises(ApiError) as caught:
            await client.request("get_me")
        self.assertEqual(caught.exception.retry_after, "7")
        self.assertEqual(len(calls), 1)

    async def test_redirect_size_and_json(self):
        replies = [httpx.Response(302, headers={"location": "https://foreign.test"}, json={"kind": "redirect"}),
                   httpx.Response(200, content=b"x" * 2048), httpx.Response(200, content=b"not JSON")]
        client = await self.client(lambda _request: replies.pop(0), max_response_bytes=1024)
        with self.assertRaises(ApiError):
            await client.request("get_me")
        for kind in ("response_too_large", "invalid_response"):
            with self.assertRaises(TransportError) as caught:
                await client.request("get_me")
            self.assertEqual(caught.exception.kind, kind)
        self.assertEqual(replies, [])

    async def test_deadline_and_cancellation_include_token_provider(self):
        async def token(_refresh):
            await asyncio.sleep(60)
        client = await self.client(lambda _: self.fail("no request"), token, timeout=0.02)
        with self.assertRaises(TransportError) as caught:
            await client.request("get_me")
        self.assertEqual(caught.exception.kind, "timeout")
        task = asyncio.create_task(client.request("get_me"))
        await asyncio.sleep(0)
        task.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await task

    async def test_invalid_parameters_fail_before_authentication(self):
        async def token(_refresh):
            self.fail("must not resolve credentials")
        client = await self.client(lambda _: self.fail("no request"), token)
        for operation, options in [("get_session", {}), ("get_session", {"path": {"session_id": ".."}}),
                                   ("get_me", {"query": {"token": "wrong"}}),
                                   ("open_session", {"body": {"workspace_id": "w1", "client_name": "test"}})]:
            with self.assertRaises(ValueError):
                await client.request(operation, **options)

    async def test_idempotent_refresh_preserves_request_and_transient_errors_are_explicit(self):
        calls = []
        async def token(refresh):
            return "fresh" if refresh else "expired"
        def handler(request):
            calls.append(request)
            return httpx.Response(401 if len(calls) == 1 else 503, json={"kind": "unavailable"})
        client = await self.client(handler, token)
        with self.assertRaises(ApiError) as caught:
            await client.request("open_session", body={"workspace_id": "w1", "client_name": "test"}, idempotency_key="stable")
        self.assertEqual(caught.exception.status, 503)
        self.assertEqual(len(calls), 2)
        self.assertEqual(calls[0].content, calls[1].content)
        for header in ("traceparent", "idempotency-key"):
            self.assertEqual(calls[0].headers[header], calls[1].headers[header])

    async def test_provider_errors_never_include_credentials(self):
        async def token(_refresh):
            raise ValueError("Bearer private-credential")
        client = await self.client(lambda _: self.fail("no request"), token)
        with self.assertRaises(TransportError) as caught:
            await client.request("get_me")
        self.assertNotIn("private-credential", str(caught.exception))


if __name__ == "__main__":
    unittest.main()
