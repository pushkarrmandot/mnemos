/**
 * Tiny ULID mint. Lexicographically sortable, 26 chars,
 * Crockford base32: 48-bit timestamp + 80 bits of randomness.
 *
 * Used for client-minted ids that must sort by creation time — toast ids and
 * chat outbox `clientId`s. Not a general-purpose UUID: the
 * monotonic-within-a-millisecond guarantee of the spec is not implemented,
 * because nothing here mints two ids inside the same tick where order matters.
 */
const ENCODING = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const TIME_LEN = 10;
const RANDOM_LEN = 16;

function encodeTime(now: number): string {
  let out = "";
  let remaining = now;
  for (let i = 0; i < TIME_LEN; i++) {
    out = ENCODING[remaining % 32] + out;
    remaining = Math.floor(remaining / 32);
  }
  return out;
}

function encodeRandom(): string {
  const bytes = new Uint8Array(RANDOM_LEN);
  crypto.getRandomValues(bytes);
  let out = "";
  for (const byte of bytes) out += ENCODING[byte % 32];
  return out;
}

export function ulid(now: number = Date.now()): string {
  return encodeTime(now) + encodeRandom();
}
