import type { Command, Item } from "./api";

export interface DeleteTarget {
  kind: "item" | "thread" | "initiative";
  id: string;
  title: string;
}

export function deleteCommand({ kind, id }: DeleteTarget): Command {
  return { [kind]: { delete: { [kind]: id } } };
}

/** Count observed relationships, including conflicting parent values that deletion detaches. */
export function deletionImpact(id: string, items: readonly Item[]): { children: number; links: number } {
  const item = items.find((one) => one.id === id);
  return {
    children: items.filter((one) => (one.conflicts.parent ?? [one.parent]).includes(id)).length,
    links: (item?.links.waits_for.length ?? 0) + (item?.links.waited_on_by.length ?? 0),
  };
}
