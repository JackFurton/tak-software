export function randomId(): string {
  // crypto.randomUUID needs a secure context, which a phone browsing http://<lan-ip> isn't.
  return Array.from(crypto.getRandomValues(new Uint8Array(8)), (b) => b.toString(16).padStart(2, "0")).join("");
}
