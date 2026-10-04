// OPS-11: keep one implementation; use prepare.mjs with explicit selectors.
import { prepare, options } from "./prepare.mjs";
try {
  const state = prepare(options(process.argv.slice(2)));
  console.log(`Prepared ${state.inputs.release} in ${state.inputs.namespace}; credentials preserved.`);
} catch (error) { console.error(error.message); process.exitCode = 78; }
