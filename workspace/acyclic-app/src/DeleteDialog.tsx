import { useEffect, useRef, useState } from "react";
import type { DeleteTarget } from "./deletion";

interface Props {
  target: DeleteTarget;
  impact: { children: number; links: number };
  onDelete: () => Promise<string | null>;
  onClose: () => void;
}

export function DeleteDialog({ target, impact, onDelete, onClose }: Props) {
  const dialog = useRef<HTMLDialogElement>(null);
  const cancel = useRef<HTMLButtonElement>(null);
  const opener = useRef<HTMLElement | null>(null);
  const sending = useRef(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    const element = dialog.current;
    const active = document.activeElement;
    if (opener.current === null && active instanceof HTMLElement)
      opener.current = active.closest("details")?.querySelector<HTMLElement>("summary") ?? active;
    element?.showModal();
    cancel.current?.focus();
    return () => element?.close();
  }, []);
  const dismiss = () => {
    if (sending.current) return;
    // Close while connected; passive cleanup after unmount cannot reliably restore native dialog focus.
    dialog.current?.close();
    if (opener.current?.isConnected) opener.current.focus({ preventScroll: true });
    onClose();
  };
  const remove = async () => {
    if (sending.current) return;
    sending.current = true;
    setBusy(true);
    dialog.current?.focus();
    setError(null);
    try {
      const failed = await onDelete();
      if (failed === null) onClose();
      else setError(failed);
    } finally {
      sending.current = false;
      setBusy(false);
    }
  };
  return (
    <dialog
      ref={dialog}
      tabIndex={-1}
      className="delete-dialog"
      aria-labelledby="delete-title"
      aria-describedby="delete-description"
      aria-busy={busy}
      onCancel={(event) => {
        event.preventDefault();
        dismiss();
      }}
      onKeyDown={(event) => {
        event.stopPropagation();
        if (event.key !== "Tab") return;
        const controls = [...event.currentTarget.querySelectorAll<HTMLElement>("button:not(:disabled), a[href]")];
        const first = controls[0];
        const last = controls.at(-1);
        const active = document.activeElement;
        if (
          first === undefined ||
          active === event.currentTarget ||
          (event.shiftKey ? active === first : active === last)
        ) {
          event.preventDefault();
          (event.shiftKey ? last : first)?.focus();
        }
      }}
      onClick={(event) => {
        if (event.target !== event.currentTarget || sending.current) return;
        const rect = event.currentTarget.getBoundingClientRect();
        if (
          event.clientX < rect.left ||
          event.clientX > rect.right ||
          event.clientY < rect.top ||
          event.clientY > rect.bottom
        )
          dismiss();
      }}
    >
      <h2 id="delete-title">Delete “{target.title}”?</h2>
      <p id="delete-description">This permanently deletes this {target.kind}. It cannot be undone.</p>
      {target.kind === "item" && (impact.children > 0 || impact.links > 0) ? (
        <div className="delete-effects">
          {impact.children > 0 ? (
            <p>
              <strong>
                {impact.children} child {impact.children === 1 ? "item will" : "items will"} be kept.
              </strong>
              <br />
              They will no longer have this item as their parent.
            </p>
          ) : null}
          {impact.links > 0 ? (
            <p>
              <strong>
                {impact.links} dependency {impact.links === 1 ? "link will" : "links will"} be removed.
              </strong>
              <br />
              Work waiting on this item may become ready.
            </p>
          ) : null}
        </div>
      ) : null}
      {target.kind === "thread" ? (
        <p className="delete-effects">
          A thread can only be deleted when it has no items and no live claim. Move its items and release its claim
          first.
        </p>
      ) : null}
      {target.kind === "initiative" ? (
        <p className="delete-effects">
          An initiative can only be deleted when it has no items. Move or delete its items first.
        </p>
      ) : null}
      {error === null ? null : (
        <p role="alert" className="danger">
          {error}
        </p>
      )}
      <div className="delete-actions">
        <button ref={cancel} type="button" disabled={busy} onClick={dismiss}>
          Cancel
        </button>
        <button type="button" className="danger" disabled={busy} onClick={remove}>
          {busy ? "Deleting…" : `Delete ${target.kind}`}
        </button>
      </div>
    </dialog>
  );
}
