# `acyclic-app`

The app: a progressive web app in React and TypeScript that the server serves to the user's browsers. It talks to the
server only through `POST /api/command`, as the paired browser's user.

```sh
mise run check:app   # typecheck, Biome's lint and format, and the unit tests
```

Node and pnpm come from mise; the app's own dependencies are pinned in `pnpm-lock.yaml`. `mise run app:install`
installs them from pnpm's store, going online only when the store lacks a package.
