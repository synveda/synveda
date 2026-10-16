/** CTX-6: clients with real implemented compact/start interfaces. */
export function supportsCheckpoint(client: string): boolean {
  return client === "claude-code" || client === "codex";
}
