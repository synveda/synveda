// OPS-12: reuse native process/HTTP tests against the installed executable.
import { execFileSync } from "node:child_process";
export function checkPackagedAuth(binary) {
  execFileSync(
    "cargo",
    [
      "test",
      "--locked",
      "-p",
      "synveda-cli",
      "--test",
      "credential_refresh",
      "--test",
      "client_platform",
    ],
    {
      cwd: new URL("..", import.meta.url),
      stdio: "inherit",
      timeout: 1200000,
      env: {
        ...process.env,
        SQLX_OFFLINE: "true",
        SYNVEDA_PACKAGED_CLI: binary,
      },
    },
  );
}
