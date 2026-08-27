#!/usr/bin/env node

import { spawnSync } from "node:child_process";

const chart = "deploy/helm/synveda";
const values = `${chart}/ci/lint-values.yaml`;

function render(extraArgs = []) {
  return spawnSync(
    "helm",
    ["template", "synveda", chart, "-f", values, ...extraArgs],
    { encoding: "utf8" },
  );
}

function requireSuccess(name, result) {
  if (result.status !== 0) {
    throw new Error(`${name} failed to render:\n${result.stderr || result.stdout}`);
  }
}

function requireRefusal(name, expected, extraArgs) {
  const result = render(extraArgs);
  if (result.status === 0) {
    throw new Error(`${name} rendered but should have been refused`);
  }
  const output = `${result.stdout}\n${result.stderr}`;
  if (!output.includes(expected)) {
    throw new Error(`${name} failed for the wrong reason:\n${output}`);
  }
}

const valid = render();
requireSuccess("minimal chart", valid);

for (const [envName, secretKey] of [
  ["SYNVEDA_KMS_KEY", "SYNVEDA_KMS_KEY"],
  ["SYNVEDA_KMS_KEY_REF", "SYNVEDA_KMS_KEY_REF"],
]) {
  const pattern = new RegExp(
    `- name: ${envName}\\n\\s+valueFrom:\\n\\s+secretKeyRef:\\n\\s+name: synveda-kms\\n\\s+key: ${secretKey}`,
  );
  if (!pattern.test(valid.stdout)) {
    throw new Error(`rendered gateway does not source ${envName} from synveda-kms/${secretKey}`);
  }
}

requireRefusal(
  "missing KMS Secret",
  "kms.existingSecret is required",
  ["--set-string", "kms.existingSecret="],
);
requireRefusal(
  "disabled extractor",
  "extractor.kind must be one of deterministic|claude|vllm",
  ["--set-string", "extractor.kind=off"],
);
requireRefusal(
  "vLLM without a model",
  "extractor.model is empty",
  [
    "--set-string",
    "extractor.kind=vllm",
    "--set-string",
    "extractor.baseUrl=http://vllm.example:8000",
  ],
);

console.log("ok: Helm startup inputs match the gateway contract and invalid extractor/KMS shapes are refused.");
