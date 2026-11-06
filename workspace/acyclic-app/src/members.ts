// A thread's items in the order its work runs: each after what it waits for inside the thread, ties by id in
// natural order. Pure, so the order is tested without a browser.

import { natural } from "./natural";

/** `members` in work order, given what each waits for; waits outside the thread, and any loop, order nothing. */
export function workOrder(members: readonly { id: string }[], waitsFor: (id: string) => readonly string[]): string[] {
  const inThread = new Set(members.map((member) => member.id));
  const waiting = new Map(members.map((member) => [member.id, waitsFor(member.id).filter((id) => inThread.has(id))]));
  // The Map puts members with no in-thread links beside the graph. Keep that group ahead of linked work in the panel,
  // even when a graph item would otherwise be ready at the same time.
  const linked = new Set<string>();
  for (const [id, firsts] of waiting) {
    if (firsts.length === 0) continue;
    linked.add(id);
    for (const first of firsts) linked.add(first);
  }
  const order: string[] = [];
  const placed = new Set([...waiting.keys()].filter((id) => !linked.has(id)).sort(natural));
  order.push(...placed);
  while (order.length < members.length) {
    const free = [...waiting.keys()].filter(
      (id) => !placed.has(id) && (waiting.get(id) ?? []).every((first) => placed.has(first)),
    );
    // A loop leaves nothing free: the rest follow by id, as nothing orders them.
    const next = (free.length > 0 ? free : [...waiting.keys()].filter((id) => !placed.has(id))).sort(natural)[0];
    if (next === undefined) break;
    order.push(next);
    placed.add(next);
  }
  return order;
}
