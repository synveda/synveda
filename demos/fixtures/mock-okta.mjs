// A directory that answers Okta's Users and Groups shapes, for
// demos/auth-5-directory-sync.sh (AUTH-5, ADR-0060).
//
// Node rather than a recorded corpus, and the distinction matters: this is
// not evidence about what Okta sends. `synveda-identity`'s connector suite
// pins the wire shapes against the vendor's documented forms; what this
// serves is a directory whose *contents the demo can change between passes*,
// which is the only way to show a leaver, a bulk departure, or a pass that
// fails half way.
//
// State lives in a JSON file the demo rewrites, and is re-read per request
// so a change takes effect on the very next pass with nothing to restart.
//
//   { "users":  [{ "id", "login", "status" }],
//     "groups": [{ "id", "name", "members": ["<user id>"] }],
//     "fail":   null | "groups" }
//
// `fail: "groups"` returns 500 from the groups collection *after* the users
// collection has answered, which is precisely an incomplete pass: users were
// listed and are present, and nothing may be concluded about who is gone.

import { createServer } from "node:http";
import { readFileSync } from "node:fs";

const [, , portArg, stateFile] = process.argv;
const port = Number(portArg);

const state = () => JSON.parse(readFileSync(stateFile, "utf8"));

const send = (res, code, body) => {
  const payload = JSON.stringify(body);
  res.writeHead(code, {
    "content-type": "application/json",
    "content-length": Buffer.byteLength(payload),
  });
  res.end(payload);
};

createServer((req, res) => {
  // The connector sends `Authorization: SSWS <token>`. Asserted rather than
  // ignored: a demo whose mock accepts anything would pass just as happily
  // against a connector that had stopped sending the credential at all.
  const auth = req.headers.authorization ?? "";
  if (!auth.startsWith("SSWS ")) {
    return send(res, 401, { errorSummary: "missing SSWS credential" });
  }

  const path = req.url.split("?")[0];
  const now = state();

  if (path === "/api/v1/users") {
    return send(
      res,
      200,
      now.users.map((user) => ({
        id: user.id,
        status: user.status ?? "ACTIVE",
        profile: { login: user.login, email: user.login },
      })),
    );
  }

  if (path === "/api/v1/groups") {
    if (now.fail === "groups") {
      return send(res, 500, { errorSummary: "the demo asked for this" });
    }
    return send(
      res,
      200,
      now.groups.map((group) => ({ id: group.id, profile: { name: group.name } })),
    );
  }

  const members = path.match(/^\/api\/v1\/groups\/([^/]+)\/users$/);
  if (members) {
    const group = now.groups.find((candidate) => candidate.id === members[1]);
    const ids = new Set(group ? group.members : []);
    return send(
      res,
      200,
      now.users
        .filter((user) => ids.has(user.id))
        .map((user) => ({
          id: user.id,
          status: user.status ?? "ACTIVE",
          profile: { login: user.login },
        })),
    );
  }

  send(res, 404, { errorSummary: `no route for ${path}` });
}).listen(port, "127.0.0.1", () => {
  process.stdout.write(`mock okta on ${port}\n`);
});
