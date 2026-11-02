// A thread's items in the order its work runs: each after what it waits for inside the thread, ties by title in
// natural order. Pure, so the order is tested without a browser.

import { natural } from "./natural";

/** `members` in work order, given what each waits for; waits outside the thread, and any loop, order nothing. */
export function workOrder(
  members: readonly { id: string; title: string }[],
  waitsFor: (id: string) => readonly string[],
): string[] {
  const inThread = new Set(members.map((member) => member.id));
  const title = new Map(members.map((member) => [member.id, member.title]));
  const byTitle = (a: string, b: string) => natural(title.get(a) ?? a, title.get(b) ?? b) || natural(a, b);
  const waiting = new Map(members.map((member) => [member.id, waitsFor(member.id).filter((id) => inThread.has(id))]));
  const order: string[] = [];
  const placed = new Set<string>();
  while (order.length < members.length) {
    const free = [...waiting.keys()].filter(
      (id) => !placed.has(id) && (waiting.get(id) ?? []).every((first) => placed.has(first)),
    );
    // A loop leaves nothing free: the rest follow by title, as nothing orders them.
    const next = (free.length > 0 ? free : [...waiting.keys()].filter((id) => !placed.has(id))).sort(byTitle)[0];
    if (next === undefined) break;
    order.push(next);
    placed.add(next);
  }
  return order;
}
