# Context-platform product evaluation

`suite.json` is the CPR-40 deterministic product and trust suite. It maps each
required scenario to an exact acceptance test and keeps the eight outcome
signals separate. `baseline.json` contains the reviewable floors and six
zero-tolerance trust bounds.

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
latencies and every scenario duration.

This deterministic suite uses the rule extractor and the test embedder, which
is lexical-only. `make eval-retrieval` remains the BGE-M3 semantic run;
`make eval-extraction-live` remains the credentialled model-extraction run;
`make eval-security` remains the 10,000-variant boundary run. Their reports and
baselines are not interchangeable with this product gate.
