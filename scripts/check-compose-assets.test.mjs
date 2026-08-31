import assert from "node:assert/strict";
import { chmodSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import test from "node:test";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const CHECKER = join(ROOT, "deploy/compose/scripts/check-compose-assets.mjs");
const PROJECT = "synveda-development-acceptance-assets";
const CONTAINER_ID = "1".repeat(64);
const NETWORK_ID = "2".repeat(64);

function fixture() {
  const scratch = mkdtempSync(join(tmpdir(), "synveda-compose-assets-"));
  const config = join(scratch, "config.json");
  const containers = join(scratch, "containers.json");
  const networks = join(scratch, "networks.json");
  const volumes = join(scratch, "volumes.json");
  const docker = join(scratch, "docker");
  writeFileSync(
    config,
    JSON.stringify({
      name: PROJECT,
      services: {
        gateway: { labels: { "com.synveda.contract": "cpr-45" } },
      },
      networks: {
        "app-backend": {
          name: `${PROJECT}_app-backend`,
          internal: true,
          ipam: {
            config: [{ subnet: "10.231.44.32/28", gateway: "10.231.44.33" }],
          },
          labels: {
            "com.synveda.contract": "cpr-45",
            "com.synveda.network": "app-backend",
          },
        },
      },
      volumes: {
        "postgres-data": {
          name: `${PROJECT}_postgres-data`,
          labels: {
            "com.synveda.contract": "cpr-45",
            "com.synveda.volume": "postgres-data",
          },
        },
      },
    }),
    { mode: 0o600 },
  );
  writeFileSync(
    containers,
    JSON.stringify([
      {
        Id: CONTAINER_ID,
        Name: `/${PROJECT}-gateway-1`,
        Config: {
          Labels: {
            "com.docker.compose.project": PROJECT,
            "com.docker.compose.service": "gateway",
            "com.docker.compose.oneoff": "False",
            "com.docker.compose.container-number": "1",
            "com.synveda.contract": "cpr-45",
          },
        },
      },
    ]),
  );
  writeFileSync(
    networks,
    JSON.stringify([
      {
        Id: NETWORK_ID,
        Name: `${PROJECT}_app-backend`,
        Driver: "bridge",
        Scope: "local",
        Internal: true,
        Attachable: false,
        Ingress: false,
        EnableIPv4: true,
        EnableIPv6: false,
        ConfigOnly: false,
        ConfigFrom: { Network: "" },
        Options: {},
        IPAM: {
          Driver: "default",
          Options: null,
          Config: [{ Subnet: "10.231.44.32/28", Gateway: "10.231.44.33" }],
        },
        Labels: {
          "com.docker.compose.project": PROJECT,
          "com.docker.compose.network": "app-backend",
          "com.synveda.contract": "cpr-45",
          "com.synveda.network": "app-backend",
        },
      },
    ]),
  );
  writeFileSync(
    volumes,
    JSON.stringify([
      {
        Name: `${PROJECT}_postgres-data`,
        Driver: "local",
        Scope: "local",
        Options: null,
        Labels: {
          "com.docker.compose.project": PROJECT,
          "com.docker.compose.volume": "postgres-data",
          "com.synveda.contract": "cpr-45",
          "com.synveda.volume": "postgres-data",
        },
      },
    ]),
  );
  writeFileSync(
    docker,
    `#!/bin/sh
set -eu
case "$1:$2" in
  container:ls) if [ "\${FAKE_ASSET_STATE:-exact}" = exact ]; then printf '${CONTAINER_ID.slice(0, 12)}\\n'; fi ;;
  container:inspect) /bin/cat "$FAKE_CONTAINER_INSPECT" ;;
  network:ls) if [ "\${FAKE_ASSET_STATE:-exact}" = exact ]; then printf '${NETWORK_ID.slice(0, 12)}\\n'; fi ;;
  network:inspect) /bin/cat "$FAKE_NETWORK_INSPECT" ;;
  volume:ls) if [ "\${FAKE_VOLUME_STATE:-exact}" = exact ]; then printf '${PROJECT}_postgres-data\\n'; fi ;;
  volume:inspect) /bin/cat "$FAKE_VOLUME_INSPECT" ;;
  *) exit 64 ;;
esac
`,
    { mode: 0o700 },
  );
  chmodSync(docker, 0o700);
  return { scratch, config, containers, networks, volumes, docker };
}

function run(state, extra = {}, mode = "existing") {
  return spawnSync(
    process.execPath,
    [
      CHECKER,
      "--config-file", state.config,
      "--project", PROJECT,
      "--docker-bin", state.docker,
      "--state", mode,
    ],
    {
      encoding: "utf8",
      env: {
        ...process.env,
        FAKE_CONTAINER_INSPECT: state.containers,
        FAKE_NETWORK_INSPECT: state.networks,
        FAKE_VOLUME_INSPECT: state.volumes,
        ...extra,
      },
    },
  );
}

test("exact Compose assets pass and stopped state requires containers and networks absent", () => {
  const state = fixture();
  try {
    const existing = run(state);
    assert.equal(existing.status, 0, existing.stderr);
    const stopped = run(
      state,
      { FAKE_ASSET_STATE: "none", FAKE_VOLUME_STATE: "exact" },
      "stopped",
    );
    assert.equal(stopped.status, 0, stopped.stderr);
    const active = run(state, {}, "stopped");
    assert.equal(active.status, 78);
    assert.match(active.stderr, /containers remain after shutdown/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("an exact-name container or network with drifted ownership is refused", () => {
  const state = fixture();
  try {
    const containerInventory = JSON.parse(String(Buffer.from(requireRead(state.containers))));
    containerInventory[0].Config.Labels["com.synveda.contract"] = "foreign";
    writeFileSync(state.containers, JSON.stringify(containerInventory));
    const badContainer = run(state);
    assert.equal(badContainer.status, 78);
    assert.match(badContainer.stderr, /project container label/);

    containerInventory[0].Config.Labels["com.synveda.contract"] = "cpr-45";
    writeFileSync(state.containers, JSON.stringify(containerInventory));
    const networkInventory = JSON.parse(String(Buffer.from(requireRead(state.networks))));
    delete networkInventory[0].Labels["com.docker.compose.project"];
    writeFileSync(state.networks, JSON.stringify(networkInventory));
    const badNetwork = run(state);
    assert.equal(badNetwork.status, 78);
    assert.match(badNetwork.stderr, /project network label/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("an extra project volume is refused before lifecycle mutation", () => {
  const state = fixture();
  try {
    const extraDocker = join(state.scratch, "docker-extra-volume");
    writeFileSync(
      extraDocker,
      requireRead(state.docker).replace(
        `printf '${PROJECT}_postgres-data\\n'`,
        `printf '${PROJECT}_postgres-data\\n${PROJECT}_foreign-data\\n'`,
      ),
      { mode: 0o700 },
    );
    chmodSync(extraDocker, 0o700);
    state.docker = extraDocker;
    const result = run(state);
    assert.equal(result.status, 78);
    assert.match(result.stderr, /volume inventory exceeded/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

test("a project network with drifted subnet or gateway is refused", () => {
  const state = fixture();
  try {
    const networkInventory = JSON.parse(requireRead(state.networks));
    networkInventory[0].IPAM.Config[0].Subnet = "10.231.99.32/28";
    writeFileSync(state.networks, JSON.stringify(networkInventory));
    const badSubnet = run(state);
    assert.equal(badSubnet.status, 78);
    assert.match(badSubnet.stderr, /runtime IPAM contract was refused/);

    networkInventory[0].IPAM.Config[0].Subnet = "10.231.44.32/28";
    networkInventory[0].IPAM.Config[0].Gateway = "10.231.44.34";
    writeFileSync(state.networks, JSON.stringify(networkInventory));
    const badGateway = run(state);
    assert.equal(badGateway.status, 78);
    assert.match(badGateway.stderr, /runtime IPAM contract was refused/);
  } finally {
    rmSync(state.scratch, { recursive: true, force: true });
  }
});

function requireRead(path) {
  return readFileSync(path, "utf8");
}
