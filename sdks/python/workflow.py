"""ADPT-4 equivalent shared-task acceptance example; see sdks/README.md."""
import asyncio
import json
import os
import secrets
import sys
from datetime import datetime, timezone
from pathlib import Path

from synveda import ApiError, Client
from synveda import operations as api


async def main():
    scenario = json.loads(Path(sys.argv[1]).read_text())

    async def bearer(_refresh):
        return Path(os.environ["SYNVEDA_TOKEN_FILE"]).read_text().strip()

    async def auditor_bearer(_refresh):
        return Path(os.environ["SYNVEDA_AUDITOR_TOKEN_FILE"]).read_text().strip()

    parent = f"00-{secrets.token_hex(16)}-{secrets.token_hex(8)}-01"
    path = {"session_id": scenario["session_id"]}
    key = scenario["run_key"] + "-python"
    async with Client(scenario["gateway"], bearer) as client, Client(scenario["gateway"], auditor_bearer) as auditor:
        session = await api.get_session(client, path=path, traceparent=parent)
        assert session.data["scope_id"] == scenario["scope_id"]
        context = await api.create_context_run(client, {"query": scenario["query"]}, path=path,
                                              idempotency_key=key + "-context", traceparent=parent)
        assert scenario["allowed_marker"] in context.data["rendered"]
        knowledge = await api.query_session_knowledge(client, {"query": scenario["query"]}, path=path, traceparent=parent)
        assert any(item["knowledge"]["id"] == scenario["knowledge_id"] for item in knowledge.data["items"])
        available = await api.list_available_skills(client, query={"scope_id": scenario["scope_id"]}, traceparent=parent)
        skill = next(entry for entry in available.data["skills"] if entry["version"]["id"] == scenario["version_id"])
        skill_path = {"id": scenario["skill_id"], "version_id": skill["version"]["id"]}
        await api.get_skill_version(client, path=skill_path, traceparent=parent)
        file = await api.get_skill_version_file(client, path={**skill_path, "path": "SKILL.md"}, traceparent=parent)
        assert isinstance(scenario["skill_marker"], str) and scenario["skill_marker"]
        assert scenario["skill_marker"] in file.data["content"]
        observed = await api.append_session_events(client, {"events": [{
            "client_event_id": key + "-skill", "event_type": "skill.loaded",
            "occurred_at": datetime.now(timezone.utc).isoformat(),
            "payload": {"skill_id": scenario["skill_id"], "version_id": skill["version"]["id"]},
        }]}, path=path, traceparent=parent)
        assert observed.data["appended"] == 1
        body = {"scope_id": scenario["scope_id"], "project_id": scenario["project_id"], "knowledge_type": "convention",
                "content": {"title": "SDK proposal", "summary": scenario["proposal_marker"],
                            "body_markdown": scenario["proposal_marker"], "confidence_permille": 900, "sensitivity": "internal"}}
        proposed = await api.create_knowledge(client, body, idempotency_key=key + "-proposal", traceparent=parent)
        assert proposed.data["outcome"] == "pending_review"
        replay = await api.create_knowledge(client, body, idempotency_key=key + "-proposal", traceparent=parent)
        assert replay.data["change_id"] == proposed.data["change_id"]
        proposal = await api.get_proposal(client, path={"id": proposed.data["change_id"]}, traceparent=parent)
        assert proposal.data["state"] == "open"
        try:
            await api.get_session(client, path={"session_id": scenario["denied_session_id"]}, traceparent=parent)
        except ApiError as error:
            assert error.status in (403, 404)
        else:
            raise AssertionError("cross-workspace Session was disclosed")
        after = await api.query_session_knowledge(client, {"query": scenario["proposal_marker"]}, path=path, traceparent=parent)
        assert scenario["proposal_marker"] not in json.dumps(after.data)
        audit = await api.list_audit_events(auditor, query={"session_id": scenario["session_id"], "limit": 100}, traceparent=parent)
        assert any(event.get("trace_id") == context.trace_id for event in audit.data["events"])
        assert scenario["allowed_marker"] not in json.dumps(audit.data)
        assert scenario["proposal_marker"] not in json.dumps(audit.data)
        print(json.dumps({"client": "python", "session_id": scenario["session_id"],
                          "context_run_id": context.data["id"], "proposal_id": proposed.data["change_id"], "trace_id": context.trace_id}))


asyncio.run(main())
