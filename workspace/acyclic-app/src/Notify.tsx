// The "Notify this device" switch: whether the server pushes to this device when something new needs the user.

import { useEffect, useState } from "react";
import { browserDevice, type PushState, pushState, turnOff, turnOn } from "./push";

const LABELS: Record<PushState, string> = {
  unsupported: "This browser can't be notified",
  blocked: "Notifications are blocked: allow them in the browser's site settings",
  off: "Notify this device",
  on: "Notifying this device",
};

export function Notify() {
  const [state, setState] = useState<PushState | "checking">("checking");
  const [failure, setFailure] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    let live = true;
    pushState(browserDevice()).then(
      (found) => live && setState(found),
      // The subscription the device holds couldn't be sent again; it is off as far as the server knows.
      () => live && setState("off"),
    );
    return () => {
      live = false;
    };
  }, []);
  if (state === "checking") return null;
  const device = browserDevice();
  const toggle = async () => {
    if (device === null) return;
    setBusy(true);
    setFailure(null);
    try {
      setState(await (state === "on" ? turnOff(device) : turnOn(device)));
    } catch (err) {
      setFailure(err instanceof Error ? err.message : String(err));
    }
    setBusy(false);
  };
  const can = state === "on" || state === "off";
  return (
    <div className="notify">
      <label>
        <input
          type="checkbox"
          data-testid="notify"
          checked={state === "on"}
          disabled={!can || busy}
          onChange={toggle}
        />{" "}
        {LABELS[state]}
      </label>
      {failure === null ? null : (
        <p className="why" role="alert">
          {failure}
        </p>
      )}
    </div>
  );
}
