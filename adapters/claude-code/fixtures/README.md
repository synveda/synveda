# Recorded fixtures (ADR-0027 decision 14)

Hook payloads and session transcripts as the harness actually emits them,
for the driver in `src/driver.mts` to replay against a mock or a live
gateway.

Both formats are internal to Claude Code and neither is a published
contract, which is the whole reason these files exist: the adapter reads
six fields and treats the rest as opaque (decision 9), and a fixture that
carries the *whole* shape is what proves the reading survives the fields
it ignores.

Recorded against Claude Code 2.1.220 (`hooks/`) and against a live
session transcript of the same build (`transcripts/`); the content is
synthetic, the shapes are not. Field-by-field:

- every hook payload carries the common envelope — `session_id`,
  `transcript_path`, `cwd`, and, when the harness has one to give,
  `prompt_id`, `permission_mode`, `agent_id`, `agent_type`, `effort`
- `SessionStart` adds `source`, `model`, and `session_title`; `Stop` adds
  `stop_hook_active`, `last_assistant_message`, `background_tasks`, and
  `session_crons`; `PreCompact` adds `trigger` and `custom_instructions`;
  `SessionEnd` adds `reason`
- transcript entries carry `parentUuid`, `isSidechain`, `type`,
  `message`, `uuid`, `timestamp`, `userType`, `cwd`, `sessionId`,
  `version`, `gitBranch`, plus `isMeta` on harness bookkeeping and
  `toolUseResult` on tool replies

The driver rewrites `transcript_path` and `cwd` to its own scratch
directory before replaying a payload — nothing here points at a real
machine, and nothing the driver runs writes outside its scratch.

The oversized-payload case is generated rather than stored: a 64 KiB
fixture is a file nobody can read, and the interesting part is the cap,
not the bytes.
