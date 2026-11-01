// A stage in the app's text: its icon, the Map's own, beside its word; and a parent's counts, each beside its stage's
// icon. The icon only repeats the word, so it is hidden from assistive technology.

import type { Counts, Stage } from "./api";
import { QUIET, STAGE_ICONS, STAGE_WORDS, stageCounts, stageCountWords } from "./stages";

export function StageIcon({ stage }: { stage: Stage }) {
  return (
    <svg
      className="stage-icon"
      data-quiet={QUIET.has(stage)}
      viewBox="0 0 16 16"
      width="16"
      height="16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
      // biome-ignore lint/security/noDangerouslySetInnerHtml: the app's own constant paths, never data.
      dangerouslySetInnerHTML={{ __html: STAGE_ICONS[stage] }}
    />
  );
}

/** A leaf's stage: icon and word. */
export function StageLabel({ stage }: { stage: Stage }) {
  return (
    <span className="stage-mark" data-stage={stage}>
      <StageIcon stage={stage} />
      {STAGE_WORDS[stage]}
    </span>
  );
}

/** A parent's leaves by stage, in words, each beside its icon; `none` when there are none to count. */
export function StageCounts({ counts, none = "No items" }: { counts: Counts; none?: string }) {
  const shown = stageCounts(counts);
  if (shown.length === 0) return <span className="stage-counts">{none}</span>;
  return (
    <span className="stage-counts">
      {shown.map(({ stage, count }, at) => (
        <span key={stage} className="stage-mark" data-stage={stage}>
          <StageIcon stage={stage} />
          <span className="count">{count}</span> {stageCountWords(stage, count)}
          {/* Read aloud as a list; seen, each count's icon already sets it apart. */}
          {at < shown.length - 1 ? <span className="visually-hidden">,</span> : null}
        </span>
      ))}
    </span>
  );
}
