# `acyclic-fixtures`

Synthetic plans for tests, benchmarks and the app spike, in the import format. A plan has goals, umbrella items,
leaf items and a few sub-items (depth 4), areas, repository records, threads, hard, soft and related links, and a
share of done work with archived subtrees. Its shape follows a real tracker's statistics without any of its content.

At about 350 items a plan has 21 threads: 15 of 4 to 46 items and 6 of 1 or 2, with about an eighth of the items in
no thread. Larger plans scale the number of threads and keep every per-item rate.

The same seed always gives the same plan:

```sh
cargo run -p acyclic-fixtures -- --items 3500 --seed 7 > plan.json
```
