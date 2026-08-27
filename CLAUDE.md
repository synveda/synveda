# CLAUDE.md

Follow [AGENTS.md](AGENTS.md). This file does not redefine project rules or
carry phase history.

For Claude Code adapter work, also read
[adapters/claude-code/README.md](adapters/claude-code/README.md), ADR-0078 and
ADR-0079.

`make claude-acceptance` is deterministic replay of authentic frames through a
live gateway. `make claude-acceptance-live` requires an installed,
authenticated proprietary client; prerequisite exit 77 means unavailable, not
pass. Never represent replay, configuration generation or a logged-out client
as live verification.
