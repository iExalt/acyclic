// Ids for new items: UUIDv7, which the tracker takes from the app, so a pending add is drawn under its real id and a
// retried send is the same add.

/** A lowercase, hyphenated UUIDv7: a 48-bit millisecond timestamp, version 7, the RFC variant, and random bits. */
export function uuidv7(now: number = Date.now()): string {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  // The timestamp fills the first six bytes, most significant first; a shift would overflow 32 bits.
  for (let at = 0; at < 6; at++) bytes[at] = Math.floor(now / 2 ** (8 * (5 - at))) % 256;
  bytes[6] = 0x70 | ((bytes[6] ?? 0) & 0x0f);
  bytes[8] = 0x80 | ((bytes[8] ?? 0) & 0x3f);
  const hex = Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}
