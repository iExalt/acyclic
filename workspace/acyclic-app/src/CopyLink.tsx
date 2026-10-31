// The Copy link button a panel's header carries: it says it copied in place, a tick for two seconds, rather than with
// a notice that would cover the panel. Where there's no clipboard, the app shows the link to copy by hand instead.

import { useEffect, useState } from "react";
import { Check, Icon, LinkIcon } from "./icons";

export function CopyLink({ onCopy }: { onCopy: () => Promise<boolean> }) {
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), 2000);
    return () => clearTimeout(timer);
  }, [copied]);
  return (
    <>
      <button
        type="button"
        className="close"
        aria-label="Copy link"
        title={copied ? "Link copied" : "Copy link"}
        data-action="copy-link"
        data-copied={copied}
        onClick={() => onCopy().then(setCopied)}
      >
        <Icon of={copied ? Check : LinkIcon} />
      </button>
      <span className="visually-hidden" role="status">
        {copied ? "Link copied" : ""}
      </span>
    </>
  );
}
