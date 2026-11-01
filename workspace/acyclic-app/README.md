# `acyclic-app`

The app: a progressive web app in React and TypeScript that the server serves to the user's browsers. It reads and
writes the tracker through `POST /api/command`, as the paired browser's user, and asks the server about notifications at
`/api/push/key`, `/api/push/subscribe` and `/api/push/unsubscribe`.

```sh
mise run check:app   # typecheck, Biome's lint and format, and the unit tests
```

Node and pnpm come from mise; the app's own dependencies are pinned in `pnpm-lock.yaml`. `mise run app:install`
installs them from pnpm's store, going online only when the store lacks a package.

`DESIGN.md` records the app's agreed design: its tokens, rules and conventions, and why.
