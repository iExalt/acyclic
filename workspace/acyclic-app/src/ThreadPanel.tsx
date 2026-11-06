// The picked thread: its progress, its items in the order their work runs, and its notes.

import { type ReactNode, useEffect, useRef, useState } from "react";
import { type Counts, failure, type Stage, type ThreadDetail, thread } from "./api";
import { Badge, lineStyle } from "./Badge";
import { CopyLink } from "./CopyLink";
import { useFocusIn } from "./focus";
import { Icon, X } from "./icons";
import type { Line } from "./lines";
import { workOrder } from "./members";
import { Prose } from "./Prose";
import { useSheet } from "./Sheet";
import { StageCounts, StageIcon } from "./Stage";

export interface ThreadPanelProps {
  id: string;
  /** The thread's line, for its badge. */
  line: Line | undefined;
  /** Its leaves by stage, as core counts them, or undefined while the plan isn't read. */
  counts: Counts | undefined;
  titleOf: (id: string) => string;
  stageOf: (id: string) => Stage | undefined;
  /** What an item waits for, to list the thread's items in the order their work runs. */
  waitsFor: (id: string) => readonly string[];
  /** Changes after every edit sent, so the panel reads the thread again. */
  version: number;
  onSelect: (id: string) => void;
  onClose: () => void;
  /** Closes it as Escape does, for a drag down on a phone: a form holding typing stays, saying how to leave it. */
  onDismiss: () => void;
  /** Copies the address that opens this again. */
  onCopyLink: () => Promise<boolean>;
}

type Loaded = { detail: ThreadDetail } | { failed: string } | null;

export function ThreadPanel({
  id,
  line,
  counts,
  titleOf,
  stageOf,
  waitsFor,
  version,
  onSelect,
  onClose,
  onDismiss,
  onCopyLink,
}: ThreadPanelProps) {
  const [loaded, setLoaded] = useState<Loaded>(null);
  const [showAll, setShowAll] = useState(false);
  // A new version, after any edit, reads the thread again.
  useEffect(() => {
    let live = true;
    thread(id).then(
      (detail) => {
        if (live) setLoaded({ detail });
      },
      (err: unknown) => {
        if (live) setLoaded({ failed: failure(err).message });
      },
    );
    return () => {
      live = false;
    };
  }, [id, version]);
  // One panel throughout, so focus put in it as it opens stays as it loads.
  const panel = useRef<HTMLElement>(null);
  const sheet = useSheet(panel, onDismiss);
  useFocusIn(panel);
  const shell = (body: ReactNode) => (
    <aside
      ref={panel}
      tabIndex={-1}
      className="inspector"
      data-testid="thread-panel"
      aria-label="Picked thread"
      aria-busy={loaded === null ? true : undefined}
      data-detent={sheet.detent}
    >
      {body}
    </aside>
  );
  if (loaded === null) return shell(null);
  if ("failed" in loaded) return shell(<p role="alert">{loaded.failed}</p>);
  const { detail } = loaded;
  const members = detail.members.length;
  const ordered = workOrder(detail.members, waitsFor);
  return shell(
    <>
      <header {...sheet.header}>
        {sheet.handle}
        <h2 className="line" style={lineStyle(line)}>
          <Badge line={line} />
          <Prose text={line?.name ?? detail.title} />
        </h2>
        <CopyLink onCopy={onCopyLink} />
        <button type="button" className="close" aria-label="Close" onClick={onClose}>
          <Icon of={X} />
        </button>
      </header>
      {/* No total beside the counts: they are of its leaves, which a total of every member, parents too, wouldn't add up to. */}
      <div className="thread-summary">
        <div className="progress">
          {counts === undefined || counts.leaves + counts.canceled === 0 ? (
            <span className="muted">No items shown</span>
          ) : (
            <StageCounts counts={counts} />
          )}
        </div>
      </div>
      {ordered.length === 0 ? null : (
        <div className="thread-items">
          <h3>
            Items <span className="count muted">{members}</span>
          </h3>
          <ul className="rows">
            {(showAll ? ordered : ordered.slice(0, ITEMS_SHOWN)).map((member) => {
              const stage = stageOf(member);
              return (
                <li key={member}>
                  <button type="button" className="row" data-select={member} onClick={() => onSelect(member)}>
                    {stage === undefined ? null : <StageIcon stage={stage} />}
                    <Prose text={titleOf(member)} />
                  </button>
                </li>
              );
            })}
          </ul>
          {showAll || ordered.length <= ITEMS_SHOWN ? null : (
            <button type="button" className="quiet more" onClick={() => setShowAll(true)}>
              {ordered.length - ITEMS_SHOWN} more
            </button>
          )}
        </div>
      )}
      {detail.notes === null ? null : (
        <div className="notes-block">
          <h3>Notes</h3>
          <p className="notes">
            <Prose text={detail.notes} notes />
          </p>
        </div>
      )}
    </>,
  );
}

/** How many of a thread's items show before the rest are asked for. */
const ITEMS_SHOWN = 8;
