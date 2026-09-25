# Context-platform product evaluation

`suite.json` is the CPR-40 deterministic product and trust suite. It maps each
required scenario to an exact acceptance test and keeps the eight outcome
signals separate. `baseline.json` contains the reviewable floors and six
zero-tolerance trust bounds.

CTX-8 adds two paired off/conservative scenarios to this same disposable
database run. Their separate `context_optimisation` report section records
complete Synveda-rendered text tokens with the declared `o200k_base` encoding,
four exact required-body checks, preview side effects and private-source
masking. The suite sets zero tolerance for critical-fact loss, private-source
leakage and preview delivery. It requires one fixed optional-content fixture
to reduce the rendered block by at least one token. These are deterministic
source fixtures, not model task-success, provider-usage or cost measurements.

CTX-6 adds a separate four-task synthetic restart corpus in
`checkpoint-tasks.json`. The exact-role gateway test compares previews with
restart off and on after two compaction boundaries, under the same Knowledge
snapshot and 1,400-token `o200k_base` Synveda text budget. It checks 12
independently declared Session facts, exact required Knowledge, checkpoint and
tail event attribution, and coverage. Both paths receive one warmup request
per task; the report records one timed request per mode and a four-sample
assisted p95. The predeclared local ceiling is 500 ms, with zero tolerance for
fact or provenance loss. This is a deterministic probe, not a model answer,
task-success or provider-cost measurement; four samples do not establish a
production latency distribution.

The shared-budget scenario repeats a checkpoint restart beside exact required
Knowledge in both governed `off` and `conservative` modes. It verifies the
checkpoint's source event and the required body under a generous allowance,
then tightens the allowance and requires the optional restart text to be
omitted without losing required Knowledge or exposing the omitted source ID.

### Prepared model task-use probe

`context-model-probe.json` fixes eight synthetic factual questions and answer
fields before any model comparison. The two existing public-API tests can export
their actual paired ContextRun text only when explicitly asked. The export is
separate from the content-free product report and belongs under ignored
`target/`; never use a real tenant in this probe.

```sh
mkdir -p target/context-model-probe
SYNVEDA_CONTEXT_OPT_MODEL_INPUT="$PWD/target/context-model-probe/optimisation-input.json" \
  SYNVEDA_DB_TEST_TASK=workspace bash scripts/db-test.sh -p synveda-gateway \
    --test context_runs conservative_required_fact_matrix_preserves_exact_task_evidence \
    -- --exact --test-threads=1
SYNVEDA_CHECKPOINT_MODEL_INPUT="$PWD/target/context-model-probe/checkpoint-input.json" \
  SYNVEDA_DB_TEST_TASK=workspace bash scripts/db-test.sh -p synveda-gateway \
    --test context_runs held_out_checkpoint_restart_tasks_preserve_critical_facts_and_provenance \
    -- --exact --test-threads=1
node scripts/prepare-context-model-probe.mjs \
  target/context-model-probe/optimisation-input.json \
  target/context-model-probe/checkpoint-input.json \
  target/context-model-probe/prompts.json
```

`prompts.json` contains 16 paired, synthetic, source-derived prompts and SHA-256
digests plus checkout revision/dirty state. Its `synthetic_context_preparation_only` label means no model was
called and no task outcome, provider usage, cost or live Claude hook was
measured. The answer key stays in the separate source rubric, outside prompts.
A later bounded provider run must use the same model/settings per pair and
send only each `prompt` string with tools disabled, then record the served
model, prompt digest and actual usage. Collect its 16 JSON
answers as `{"schema_version":1,"model_id":"...","responses":[...]}`;
each response names `family`, `task_id`, `variant`, the corresponding
`prompt_sha256`, and an `answer` object with exactly the rubric's fields. Then
score the fixed rubric with:

```sh
node scripts/score-context-model-probe.mjs \
  target/context-model-probe/prompts.json \
  target/context-model-probe/answers.json \
  target/context-model-probe/score.json
```

The scorer rejects missing, repeated or mismatched answers and reports
per-task regressions. It labels manually imported answers unverified, so even
its `PASS` is not live provider evidence. This factual task-use probe remains
narrower than coding-task success or a native compact/resume run.

Run the definition-only CI gate with:

```sh
make eval-check
```

Run the database-backed product suite and write both machine-readable and
human-readable reports under `target/product-evaluation/` with:

```sh
make eval-product
```

The runner requires `DATABASE_URL`; it fails if the PulseBoard test skips and
therefore cannot turn an unavailable database into green evidence. It records
the exact git revision, retrieval/index/embedding identity, independently
persisted retrieved/selected/injected/feedback counts, token use, ContextRun
latencies, paired CTX-8 and CTX-6 evidence and every scenario duration.

This deterministic suite uses the rule extractor and the test embedder, which
is lexical-only. `make eval-retrieval` remains the BGE-M3 semantic run;
`make eval-extraction-live` remains the credentialled model-extraction run;
`make eval-security` remains the 10,000-variant boundary run. Their reports and
baselines are not interchangeable with this product gate.
