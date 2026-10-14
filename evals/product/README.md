# Context-platform product evaluation

`suite.json` is the CPR-40 deterministic product and trust suite. It maps each
required scenario to an exact acceptance test and keeps the eight outcome
signals separate. `baseline.json` contains the reviewable floors and six
zero-tolerance trust bounds.

The source suite also includes saved-delivery admission, typed retry-cap conflict
review and tight-budget exception preservation. These prove exact public-API
behavior on fictional evidence, with unchanged quality floors. The focused
`bash demos/mem-8-saved-delivery.sh` runs all saved-replay modes, independent private
provenance, checkpoint source disposal, retry caps 3/5 and the payment exception
on a fresh ordinary-role PostgreSQL fixture. It is deterministic API replay and
does not establish native handoff, full source forgetting or model use.
The suite now has 31 scenarios, including private all-mode lineage, physical
delivery-copy erasure, authored channel changes and immutable Skill version
changes. These checks retain the original quality floors and trust tolerances.
The saved-delivery demo runs ten focused cases and never infers Skill execution
from an advertisement.
Exact-policy replay additionally proves that a committed Alice-only denial
applies while the process cache still holds version 1, and Bob retains the exact
authorized receipt. The request-local snapshot does not update the process cache
or establish the later atomic admission/removal boundary.

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

After an explicit total-spend approval and a locally authenticated Claude Code
client, the opt-in runner invokes each prompt as a separate no-tools,
non-persistent print call. It divides the approved cap across 16 calls, checks
the returned model and per-call usage, and writes private answer and score
files under `target/`. For a USD $5.00 approval and the `sonnet` CLI selector:

```sh
SYNVEDA_CONFIRM_MODEL_SPEND=ctx-probe:5.00:16 \
  node scripts/run-context-model-probe.mjs \
    target/context-model-probe/prompts.json sonnet 5.00 \
    target/context-model-probe/live
```

The runner refuses a stale or dirty prompt manifest, absent confirmation or
authentication, model drift, missing usage, malformed JSON and a reported
cost above the per-call cap. It retains a labelled partial result if a later
call fails. Its final score measures factual answers in this synthetic
questionnaire only; native hooks, coding-task success, subscription invoices
and net savings require separate evidence.

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

### Context and memory qualification

The 40-scenario inventory includes grounded cross-interface handoff, complete
caller-reported host allowance, explicit keep-separate review for equal-byte
independent references, authenticated/frozen Procedure qualification, baseline
erasure, the eight-attempt bound, source-health precedence and the integrated
[Iris domain lifecycle](../../docs/integrations/iris-domain.md). Existing six
zero-tolerance trust gates and baseline floors remain unchanged.

`health-diagnosis.mjs` is a separate owned-fixture oracle. Its public-API test
freezes delivery before exact-ID inspection, retains actual retrieval limitations,
injects qualifier loss only into a diagnostic in-memory copy and observes a
controlled failure despite correct context. Production representation coverage,
recall and causal attribution remain unknown. Oracle labels are never retrieval
inputs. Run `node --test evals/product/health-diagnosis.test.mjs` for the oracle
boundary and the exact-role `context_health` suite for its actual source evidence.

`iris-domain-evidence.json` records a real CLI preview's local `o200k_base` count,
owned Git revision, six published source records, three required revisions, two
fixed passing cases, successful cleanup and the independent survivor. It does
not record source content, credentials or private reasoning. Provider usage,
cost and native vendor execution are explicit unavailable/not-run fields. The
runner refuses a missing Iris receipt or a changed qualification/accounting
boundary; a silently skipped database test cannot pass.
