import assert from "node:assert/strict";
import test from "node:test";

import {
  ipv4Interval,
  networkPreflightFindings,
} from "../deploy/compose/scripts/check-network-preflight.mjs";

const project = "synveda-development-acceptance-network";
const pool = "10.231.44.0/24";

function network({
  id = "network-id",
  name = "foreign",
  subnet = "10.1.0.0/24",
  gateway = "10.1.0.1",
  ipRange,
  labels = {},
  internal = false,
} = {}) {
  return {
    Id: id,
    Name: name,
    Driver: "bridge",
    Scope: "local",
    Internal: internal,
    Attachable: false,
    Ingress: false,
    EnableIPv4: true,
    EnableIPv6: false,
    ConfigOnly: false,
    ConfigFrom: { Network: "" },
    Options: {},
    Labels: labels,
    IPAM: {
      Driver: "default",
      Options: null,
      Config: [{ Subnet: subnet, Gateway: gateway, ...(ipRange ? { IPRange: ipRange } : {}) }],
    },
  };
}

test("IPv4 intervals require canonical network addresses", () => {
  assert.deepEqual(ipv4Interval("10.231.44.0/24"), {
    start: 182922240,
    end: 182922495,
  });
  assert.equal(ipv4Interval("10.231.44.1/24"), undefined);
  assert.equal(ipv4Interval("10.231.044.0/24"), undefined);
  assert.equal(ipv4Interval("2001:db8::/64"), undefined);
});

test("an unused selected pool passes", () => {
  assert.deepEqual(networkPreflightFindings([], project, pool), []);
  assert.deepEqual(networkPreflightFindings([network()], project, pool), []);
});

test("foreign overlap and stale project networks are refused", () => {
  assert.deepEqual(
    networkPreflightFindings(
      [network({ subnet: "10.231.44.128/25", gateway: "10.231.44.129" })],
      project,
      pool,
    ),
    ["selected pool overlaps a foreign network"],
  );
  assert.deepEqual(
    networkPreflightFindings(
      [
        network({
          name: `${project}_obsolete`,
          labels: { "com.docker.compose.project": project },
        }),
      ],
      project,
      pool,
    ),
    ["stale project network was refused"],
  );
});

test("an exact retained project network is accepted and drift is refused", () => {
  const exact = network({
    name: `${project}_identity-backend`,
    subnet: "10.231.44.0/28",
    gateway: "10.231.44.1",
    ipRange: "10.231.44.8/29",
    internal: true,
    labels: {
      "com.docker.compose.project": project,
      "com.docker.compose.network": "identity-backend",
      "com.synveda.contract": "cpr-45",
      "com.synveda.network": "identity-backend",
    },
  });
  assert.deepEqual(networkPreflightFindings([exact], project, pool), []);
  assert.deepEqual(
    networkPreflightFindings([
      network({
        name: `${project}_identity-backend`,
        subnet: "10.99.0.0/24",
        gateway: "10.99.0.1",
      }),
    ], project, pool),
    ["project network contract drifted"],
  );
  assert.deepEqual(
    networkPreflightFindings(
      [{ ...exact, Labels: { ...exact.Labels, "com.synveda.contract": "wrong" } }],
      project,
      pool,
    ),
    ["project network contract drifted"],
  );
  for (const drifted of [
    { ...exact, Driver: "macvlan" },
    { ...exact, Scope: "swarm" },
    { ...exact, Internal: false },
    { ...exact, Attachable: true },
    { ...exact, Ingress: true },
    { ...exact, EnableIPv6: true },
    { ...exact, Options: { "com.docker.network.bridge.enable_icc": "true" } },
    { ...exact, IPAM: { ...exact.IPAM, Driver: "custom" } },
  ]) {
    assert.deepEqual(
      networkPreflightFindings([drifted], project, pool),
      ["project network contract drifted"],
    );
  }
});
