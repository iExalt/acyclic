// Natural order, as the CLI sorts: runs of ASCII digits compare as numbers, so "phase-2" sorts before "phase-10", and
// the rest compares with ASCII case folded, so "step 1" sorts beside "Step 1". Strings that differ only in leading
// zeros or case, such as "02" and "2", fall back to plain order, by UTF-8 bytes as the CLI compares, so the order is
// total and the same on every screen.

const encoder = new TextEncoder();

/** A run of a string: digits without their leading zeros, or anything else, ASCII case folded. */
type Run = { number: string } | { text: Uint8Array };

function runs(text: string): Run[] {
  return (text.match(/[0-9]+|[^0-9]+/g) ?? []).map((run) =>
    /^[0-9]/.test(run)
      ? { number: run.replace(/^0+/, "") }
      : { text: encoder.encode(run.replace(/[A-Z]/g, (letter) => letter.toLowerCase())) },
  );
}

function bytes(a: Uint8Array, b: Uint8Array): number {
  for (let at = 0; at < Math.min(a.length, b.length); at++) {
    const diff = (a[at] ?? 0) - (b[at] ?? 0);
    if (diff !== 0) return diff;
  }
  return a.length - b.length;
}

function run(a: Run, b: Run): number {
  // Digits sort before anything else; a longer number is a larger one.
  if ("number" in a && "number" in b) {
    return a.number.length - b.number.length || (a.number < b.number ? -1 : a.number > b.number ? 1 : 0);
  }
  if ("number" in a) return -1;
  if ("number" in b) return 1;
  return bytes(a.text, b.text);
}

/** Compares two strings in natural order: negative when `a` comes first. */
export function natural(a: string, b: string): number {
  const [x, y] = [runs(a), runs(b)];
  for (let at = 0; at < Math.min(x.length, y.length); at++) {
    const order = run(x[at] as Run, y[at] as Run);
    if (order !== 0) return order;
  }
  return x.length - y.length || bytes(encoder.encode(a), encoder.encode(b));
}
