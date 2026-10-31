// Over the Map home, while anything needs the user: how much, by what to do, and a way into the list. Dismissed for
// the session; the Needs you button still counts.

import { useState } from "react";
import { Icon, X } from "./icons";
import { countText, GROUPS, groupOf, type Need } from "./needs";

const VERBS: Record<string, string> = { approve: "to approve", answer: "to answer", unblock: "to unblock" };

export function NeedsStrip({ needs, onOpen }: { needs: Need[]; onOpen: () => void }) {
  const [dismissed, setDismissed] = useState(false);
  if (dismissed || needs.length === 0) return null;
  const parts = GROUPS.map((one) => ({
    ...one,
    count: needs.filter((need) => groupOf(need.kind) === one.group).length,
  }))
    .filter((one) => one.count > 0)
    .map((one) => `${one.count} ${VERBS[one.group]}`);
  return (
    <div className="needs-strip" data-testid="needs-strip">
      <span className="badge needs-mark" aria-hidden="true">
        !
      </span>
      <button type="button" className="strip-open" onClick={onOpen}>
        <strong>{countText(needs.length)} need you</strong>
        <span className="muted">{parts.join(", ")}</span>
      </button>
      <button type="button" className="close" aria-label="Dismiss for now" onClick={() => setDismissed(true)}>
        <Icon of={X} size={16} />
      </button>
    </div>
  );
}
