# `acyclic-fixtures`

Synthetic plans in the import format, for tests, benchmarks, the browser journeys and the app's development script. A
plan has root items (epics under one initiative), umbrella items, leaf items and a few sub-items (depth 4), repository
records, threads, waits-for links, and a share of done work. Its shape follows a real tracker's statistics without any
of its content.

At about 350 items a plan has 21 threads: 15 of 4 to 46 items and 6 of 1 or 2, with about an eighth of the items in
no thread. Larger plans scale the number of threads and keep every per-item rate.

Threads never wait on each other in a cycle, whatever the seed: a link is kept only when it runs from a later thread to
an earlier one or within one, as a real tracker's seldom do. A test that needs a cycle makes one with two edits. Plans
of 50 items and more keep the shape in proportion: a share of threads, some done work, some parked work and every depth.

The same seed always gives the same plan:

```sh
cargo run -p acyclic-fixtures -- --items 3500 --seed 7 > plan.json
```
