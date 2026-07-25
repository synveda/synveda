/**
 * A scriptable stand-in for the gateway, shared by the handler suite and
 * the recorded-payload driver (ADR-0027 decision 14).
 *
 * It is deliberately dumb: it records what arrived and answers whatever
 * the case told it to. Everything worth asserting about the adapter under
 * an odd gateway — a 401, a degradation header, a rejected batch, a
 * duplicate — is expressible as a reply, so the cases stay readable and
 * this file stays boring.
 */

import { once } from "node:events";
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";

/** One request as the gateway saw it. */
export interface RecordedRequest {
  path: string;
  body: Record<string, unknown>;
  /** The bearer the adapter presented, verbatim. */
  authorization?: string;
}

export interface Reply {
  status: number;
  body?: unknown;
  headers?: Record<string, string>;
}

export type Responder = (request: RecordedRequest, index: number) => Reply;

export interface MockGateway {
  url: string;
  requests: RecordedRequest[];
  close: () => Promise<void>;
}

export async function startGateway(respond: Responder): Promise<MockGateway> {
  const requests: RecordedRequest[] = [];
  const server = createServer((request: IncomingMessage, response: ServerResponse) => {
    let raw = "";
    request.on("data", (piece: unknown) => {
      raw += String(piece);
    });
    request.on("end", () => {
      const recorded: RecordedRequest = {
        path: request.url ?? "",
        body: parseBody(raw),
        authorization: request.headers.authorization,
      };
      const index = requests.length;
      requests.push(recorded);
      const reply = respond(recorded, index);
      response.writeHead(reply.status, {
        "content-type": "application/json",
        ...reply.headers,
      });
      response.end(JSON.stringify(reply.body ?? {}));
    });
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const address = server.address();
  const port = typeof address === "object" && address !== null ? address.port : 0;
  return {
    url: `http://127.0.0.1:${String(port)}`,
    requests,
    close: async () => {
      server.close();
      await once(server, "close");
    },
  };
}

function parseBody(raw: string): Record<string, unknown> {
  try {
    const parsed: unknown = JSON.parse(raw);
    if (parsed !== null && typeof parsed === "object") return parsed as Record<string, unknown>;
  } catch {
    // A case that sends unparseable JSON is asserting something else.
  }
  return {};
}
