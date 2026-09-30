// Disposable fault endpoint: hold real Capture work until its worker is stopped.
import { createServer } from "node:http";
let blocked = true, calls = 0, cancelled = 0, failNext = 0, failed = 0;
const peers = [];
const pending = new Set();
createServer((request, response) => {
  if (request.url === "/stats") return response.end(JSON.stringify({ blocked, calls, active: pending.size, cancelled, failed, peers }));
  if (request.url === "/block" && request.method === "POST") {
    if (pending.size) { response.writeHead(409); return response.end(); }
    blocked = true;
    return response.end("blocked");
  }
  if (request.url === "/fail-next" && request.method === "POST") {
    if (pending.size || failNext) { response.writeHead(409); return response.end(); }
    failNext = 1;
    return response.end("armed");
  }
  if (request.url === "/release" && request.method === "POST") {
    blocked = false;
    for (const finish of pending) finish();
    pending.clear();
    return response.end("released");
  }
  if (request.url === "/release-current" && request.method === "POST") {
    // Drain surviving calls while a lost owner's lease expires; later calls
    // remain blocked so a fenced retry can be inspected before completion.
    for (const finish of pending) finish();
    pending.clear();
    return response.end("released current calls");
  }
  if (request.url !== "/v1/chat/completions" || request.method !== "POST") {
    response.writeHead(404); return response.end();
  }
  calls++;
  if (peers.length < 8) peers.push(request.socket.remoteAddress);
  // The fixture never records the incoming Session payload or credentials.
  request.resume();
  if (failNext) {
    failNext--;
    failed++;
    response.writeHead(503);
    return response.end("provider temporarily unavailable");
  }
  const finish = () => response.end(JSON.stringify({ model: "ops11-interruption", choices: [{ message: { content: JSON.stringify({ candidates: [{
    knowledge_type: "convention", title: "Interrupted worker recovery", body_markdown: "Remember: interrupted workers recover their fenced Capture batch.",
    summary: "Interrupted Capture recovery", confidence: 0.95, sensitivity: "internal", tags: ["ops11-recovery"],
  }] }) } }] }));
  if (blocked) pending.add(finish); else finish();
  response.on("close", () => {
    if (!response.writableFinished) cancelled++;
    pending.delete(finish);
  });
}).listen(8088, "0.0.0.0");
