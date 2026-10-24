// The Map's layouts in the browser: ELK runs in its own worker, a box per request, and only the latest view's
// layout is kept, since toggles in quick succession queue layouts that a newer one has made stale.

import ElkApi from "elkjs/lib/elk-api.js";
import workerUrl from "elkjs/lib/elk-worker.min.js?url";
import { elkLayBox } from "./elk";
import { type LayBox, LayoutCache, type Placed, place } from "./layout";
import type { View } from "./model";

/** Lays out views one after another, answering each request with its layout, or null once a newer one was made. */
export class Layouts {
  private readonly cache = new LayoutCache();
  private latest = 0;

  constructor(private readonly layBox: LayBox) {}

  async layout(view: View): Promise<Placed | null> {
    const request = ++this.latest;
    const placed = await place(view, this.cache, this.layBox);
    return request === this.latest ? placed : null;
  }
}

/**
 * The app's layouts, with ELK in a classic worker of its own. `?engine=inline` runs ELK on the page's own thread
 * instead, loaded only then, so a timing run can tell ELK's own time from what the worker's messages add.
 */
export function browserLayouts(): Layouts {
  if (new URLSearchParams(location.search).get("engine") === "inline") {
    const elk = import("elkjs/lib/elk.bundled.js").then(({ default: Elk }) => new Elk());
    return new Layouts(elkLayBox({ layout: async (graph) => (await elk).layout(graph) }));
  }
  const elk = new ElkApi({ workerFactory: () => new Worker(workerUrl) });
  return new Layouts(elkLayBox(elk));
}
