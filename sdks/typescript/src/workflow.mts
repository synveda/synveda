/** ADPT-4: runnable shared-task acceptance example; see sdks/README.md. */
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { randomBytes } from "node:crypto";
import { ApiError, Client } from "./client.mjs";

const scenario = JSON.parse(await readFile(process.argv[2] ?? "", "utf8"));
const bearer = async () => (await readFile(process.env.SYNVEDA_TOKEN_FILE ?? "", "utf8")).trim();
const auditorBearer = async () => (await readFile(process.env.SYNVEDA_AUDITOR_TOKEN_FILE ?? "", "utf8")).trim();
const client = new Client(scenario.gateway, bearer);
const auditor = new Client(scenario.gateway, auditorBearer);
const traceparent = `00-${randomBytes(16).toString("hex")}-${randomBytes(8).toString("hex")}-01`;
const path = { session_id: scenario.session_id };
const key = `${scenario.run_key}-typescript`;

const session = await client.request("get_session", { path, traceparent });
assert.equal(session.data.scope_id, scenario.scope_id);
const context = await client.request("create_context_run", { path, body: { query: scenario.query },
  idempotencyKey: `${key}-context`, traceparent });
assert.ok(context.data.rendered?.includes(scenario.allowed_marker), "allowed context must be delivered");
const knowledge = await client.request("query_session_knowledge", { path, body: { query: scenario.query }, traceparent });
assert.ok(knowledge.data.items.some((item) => item.knowledge.id === scenario.knowledge_id));

const available = await client.request("list_available_skills", { query: { scope_id: scenario.scope_id }, traceparent });
const skill = available.data.skills.find((entry) => entry.version.id === scenario.version_id);
assert.ok(skill, "only an approved bound version may be selected");
const skillPath = { id: scenario.skill_id, version_id: skill.version.id };
await client.request("get_skill_version", { path: skillPath, traceparent });
const file = await client.request("get_skill_version_file", { path: { ...skillPath, path: "SKILL.md" }, traceparent });
assert.ok(file.data.content.includes("# Code Review"));

const observed = await client.request("append_session_events", { path, traceparent, body: { events: [{
  client_event_id: `${key}-skill`, event_type: "skill.loaded", occurred_at: new Date().toISOString(),
  payload: { skill_id: scenario.skill_id, version_id: skill.version.id },
}] } });
assert.equal(observed.data.appended, 1);
const body = { scope_id: scenario.scope_id, project_id: scenario.project_id, knowledge_type: "convention" as const,
  content: { title: "SDK proposal", summary: scenario.proposal_marker, body_markdown: scenario.proposal_marker,
    confidence_permille: 900, sensitivity: "internal" as const } };
const proposed = await client.request("create_knowledge", { body, idempotencyKey: `${key}-proposal`, traceparent });
assert.equal(proposed.data.outcome, "pending_review", "the configured policy requires review");
const replay = await client.request("create_knowledge", { body, idempotencyKey: `${key}-proposal`, traceparent });
assert.equal(replay.data.change_id, proposed.data.change_id);
const proposal = await client.request("get_proposal", { path: { id: proposed.data.change_id }, traceparent });
assert.equal(proposal.data.state, "open");
await assert.rejects(client.request("get_session", { path: { session_id: scenario.denied_session_id }, traceparent }),
  (error) => error instanceof ApiError && [403, 404].includes(error.status));
const after = await client.request("query_session_knowledge", { path, body: { query: scenario.proposal_marker }, traceparent });
assert.ok(!JSON.stringify(after.data).includes(scenario.proposal_marker), "pending content is not Knowledge");
const audit = await auditor.request("list_audit_events", { query: { session_id: scenario.session_id, limit: 100 }, traceparent });
assert.ok(audit.data.events.some((event) => event.trace_id === context.traceId));
assert.ok(!JSON.stringify(audit.data).includes(scenario.allowed_marker));
assert.ok(!JSON.stringify(audit.data).includes(scenario.proposal_marker));
console.log(JSON.stringify({ client: "typescript", session_id: scenario.session_id,
  context_run_id: context.data.id, proposal_id: proposed.data.change_id, trace_id: context.traceId }));
