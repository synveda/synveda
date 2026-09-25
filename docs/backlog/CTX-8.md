# CTX-8: Governed context optimisation

## Problem and evidence

At `ff8f52b` the current planner measures `ceil(chars / 4)` as if it were a conservative token count, folds authored pack whitespace, labels a title-only fallback a summary, and spends a fixed authored share before Knowledge. The final rendered estimate is checked, but it cannot represent an exact tokenizer, the model boundary, or the byte limits of an unknown host request. ContextRun creation is a delivery record, so it cannot serve as a preview. The CLI can inspect and publish governed Configuration but has no context preview or delivery-inspection command. The existing console inspector exposes Knowledge trace detail without an optimisation policy or excerpt status. The 2026-09-25 baseline passed `make check-fast` and retrieval unit tests; the Docker socket was absent, so the database/evaluation/Compose baseline was not run.

## Scope

- Add an opt-in conservative mode to governed Configuration, with `off` preserving the present delivery comparison path. Do not expose balanced mode until a measured backend exists.
- Count the complete rendered Synveda contribution after serialisation; identify exact supported encodings and label unknown-model counts as estimates. Retain independent byte/item limits.
- Select bounded, immutable and freshly authorised source revisions; preserve formatting, exact provenance, source hashes and delivered-output hashes. Emit genuine excerpts with source spans and title references as indexes, not summaries. An explicit required input must fail when it cannot fit.
- Share one allowance across Knowledge, authored packs, Skills and their wrappers, with deterministic selection and safe within-request deduplication.
- Extend the existing context API, authenticated CLI, console inspector and deterministic evaluation/demo path. Preview must not create a delivery/usage record.
- Keep learned compression as a benchmark-only, opt-in go/no-go investigation after conservative mode and CTX-6 have reproducible quality and lifecycle evidence.

## Non-goals

- No host-wide context-window or billing guarantee from an MCP-only integration.
- No remote model service, Python, GPU, model download, unapproved egress or new orchestration subsystem for conservative mode.
- No cross-turn suppression without a tested host acknowledgement/generation contract.
- No rewriting opaque host compaction state or treating Session evidence as approved Knowledge.

## Architecture seam

ADR-0118 proposes one governed mode field on the existing Configuration document, one reusable rendered-text counter and a pure passage selector. The session-scoped context planner remains the only delivery path. Exact source reads still cross Cedar and forced RLS before optimisation. Trace disclosure remains ADR-0084's independently re-authorised view. CTX-6 owns checkpoint/restart and will reuse shared accounting without being folded into this feature.

## Acceptance criteria

- The same authorised task, immutable revisions and configuration produce stable conservative output; the final rendered text respects an exact declared encoding budget, or reports an explicitly estimated unknown-model count.
- Off remains a runnable comparison/rollback path. Conservative mode needs no optional provider.
- Duplicate, conflict, superseded, identifier, negation, code whitespace, multilingual, tiny-budget and long-wrapper fixtures retain required facts and source attribution or return a typed insufficient-budget outcome.
- Preview creates no context-run delivery/usage row, while a real run retains a content-free audit event and re-authorised diagnostics that expose no denied address, title, count or omission reason.
- CLI and console exercise the same public API and show mode, effective budget, selected versions, excerpt/index status, allowed omission reasons and estimated/observed distinction.
- Paired evaluation compares governed off and conservative delivery on the same fixture snapshot, records per-task regressions and does not claim cost savings without provider usage and versioned rates.

## Required tests

- Unit/property cases for counting and rendered cap, paragraph/code spans, Unicode, exact hash separation, deterministic ties, dedup scope and bounded work.
- Fresh-database gateway cases for configuration mutation, allow/deny/revoke, preview side effects, trace masking and idempotent real delivery.
- Existing product/security suite plus four representative paired tasks; source Compose demo and acceptance on the documented disposable deployment.

## Rollout and rollback

Keep built-in documents and existing versions in off mode. Permit conservative mode only through a governed configuration version/binding after its deterministic gates pass; rollback rebinds or publishes off while retaining immutable evidence.

## Dependencies

The local exact tokenizer must have verified encoding compatibility and permissive code/data licences. The core path must stay within its current dependency direction and installation footprint. Docker-backed quality/security/Compose gates require a working Docker engine and isolated fixture. Learned candidates require held-out task evidence, separately verified weights/licences and an approved spend/egress policy.

### Current implementation and evidence (2026-09-25)

The source checkout now has the governed `off`/`conservative` Configuration selector, a local exact `o200k_base` rendered-text counter when that encoding is declared, labelled reference estimates otherwise, a bounded semantic passage selector, exact source spans and separately hashed delivered text. The gateway preview reuses current-revision Cedar/RLS reads without creating a ContextRun; real delivery retains its ContextRun and content-free audit. CLI preview/inspect/detail and the existing console Context Inspector call the public API. The source deployment guide contains the login, Configuration and inspection sequence. No extra model service is on the conservative path.

The fresh-database context-run tests, context-pack tests, CLI suite, console tests and generated-contract checks pass. The 23-scenario deterministic product suite includes the CTX-8 privacy, stale-revision, preview-delivery and exact required-body gates. It also checks checkpoint restart beside required Knowledge in both governed optimisation modes: a generous budget retains exact body and checkpoint source attribution, while a tight budget omits optional restart text and its source ID. A fresh product-evaluation run recorded off 710 and conservative 481 local `o200k_base` rendered-text tokens, a reduction of 229; generated identifiers can change the absolute wrapper count between runs. The four paired required-body cases retained their exact task text with no token increase. This measures Synveda's rendered text only, not a host prompt, provider usage, task success or money. The CTX-8 source demo passes against a fresh isolated database. ADR-0118 records the implemented decision.

The feature remains open because the full source Compose/browser acceptance and held-out task-quality and latency comparisons have not run with this change. On 2026-09-25, the stopped `synveda-development-acceptance-interop` project's device-only hosts receipt was renewed through the documented exact-confirmation helper, and the managed hostname block was handed to a fresh `synveda-development-acceptance-ctx8-25` project on `10.231.78.0/24`. Its hosts status and resolver checks passed; the interop Postgres volume remained intact. After the failed build, the original hosts bytes and interop ownership were restored and rechecked. No CTX-8 acceptance containers or volumes were created.

The first `make compose-acceptance` stopped in the source image build when Rustup tried to sync the declared toolchain through `static.rust-lang.org`; Debian package-index requests also stalled in that attempt. The official Rust base image lacks the rustfmt/Clippy components named for source development. The product Dockerfile now checks that the source channel matches the pinned base-image compiler and selects its already installed Rust 1.96 toolchain for compilation. A network-disabled Buildx probe confirmed the installed compiler and Cargo, and a real product Rust-stage build passed the prior Rustup failure point to Cargo dependency resolution. Cargo then waited at `index.crates.io`; separate host and ephemeral-container probes timed out for both `index.crates.io` and `static.crates.io`, so the diagnostic build was stopped. A repeat direct host HTTPS probe on 2026-09-25 resolved both official Cargo hosts but timed out before connecting; forcing the index probe to either address family did not recover it. Debian's index later responded from both. Full image construction, browser and CLI login remain untested in this source deployment. Unknown receiving models remain estimates; no host-wide or billable-token claim is supported. The next action is to restore outbound access to Cargo's official index and archive endpoints under the existing build trust boundary, then repeat the documented isolated hostname handoff and fresh Compose/browser acceptance. Collect held-out paired tasks and actual host/provider usage where supported. Keep conservative opt-in and off as rollback until those gates pass. CTX-6 checkpoint/restart and the optional learned-compressor decision follow their own evidence gates.

### Optional learned-compressor gate (2026-09-25)

**No-go for promotion or an installable backend.** The deterministic suite establishes the conservative path only; no held-out paired Synveda task, latency, safety or net-cost measurement exists for a learned compressor. A research-only comparison found [LLMLingua-2](https://github.com/microsoft/LLMLingua) is a Python/PyTorch token classifier, with an [Apache-2.0 BERT multilingual MeetingBank model](https://huggingface.co/microsoft/llmlingua-2-bert-base-multilingual-cased-meetingbank) and a separate [MIT XLM-R large model](https://huggingface.co/microsoft/llmlingua-2-xlm-roberta-large-meetingbank). Their model cards describe meeting-transcript-oriented training, not Synveda's protected code/configuration cases; the large model files add a substantial download. Code and weights have separate licences. [SWE-Pruner](https://github.com/Ayanami1314/swe-pruner/blob/public/swe-pruner/README.md) is a more relevant coding-context candidate but its published inference path requires Python 3.12, PyTorch, CUDA and Flash Attention, and its [0.6B model card](https://huggingface.co/ayanami-kitasan/code-pruner) recommends the repository's custom service. None belongs in ordinary Compose startup.

No model was downloaded, invoked or added to the product. The optional benchmark gate remains a future isolated experiment: predeclare protected facts, task-quality and p95 latency tolerances; run LLMLingua-2 on prose and SWE-Pruner on code against held-out paired tasks with current Cedar-authorised snapshots; count optimizer time/cost and validate every output. A public balanced mode remains absent. LongLLMLingua and RTK remain comparison candidates only. The next action is to obtain approved held-out task fixtures, an allowed model-serving environment and explicit egress/spend boundaries before testing candidate weights.
