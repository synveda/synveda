#!/usr/bin/env node

// One process-independent monotonic clock sample for the POSIX lifecycle
// selector. Integer seconds match the deadline runner's public granularity.
process.stdout.write(`${process.hrtime.bigint() / 1_000_000_000n}\n`);
