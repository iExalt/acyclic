# Changelog

## [0.3.0](https://github.com/iExalt/acyclic/compare/v0.2.0...v0.3.0) (2026-10-24)


### ⚠ BREAKING CHANGES

* claims move from items to threads: `claim` and `release` take a thread and return `{thread, …}`, the server's claim body names `thread`, `done` no longer releases, and claims older versions made on items are ignored. `ready` returns threads with their items; readiness drops `rank`, adds `unthreaded`, and a busy resource names its `thread`; needs gains an `unthreaded` entry, its contest, clock-ahead and unknown-run entries carry `thread`, and resource-clash holders are (thread, holder) pairs. Follows links can no longer be added, and links drop `follows` and `followed_by`. `proposals` in write results holds `{id, field}` objects, and `graph --output <FILE>` is now `graph --file <FILE>`. Older binaries ignore a claim on a thread but can't see one either, so don't mix this build with older ones that still claim items.

### Features

* **app:** add, move and link items on the Map, with an inspector ([a04bc50](https://github.com/iExalt/acyclic/commit/a04bc5054e490f70ef045d3d7ac816ee2be63185))
* **app:** draw the Map, laid out box by box, with Cytoscape.js ([2639fe9](https://github.com/iExalt/acyclic/commit/2639fe9b9102d46bce20329ac9a92b874ed2ecbe))
* **app:** draw threads as numbered, coloured lines, in a quieter shell ([8246fbd](https://github.com/iExalt/acyclic/commit/8246fbd33fdb5038fb703e8921757594c58ee552))
* **app:** edit fields, settle conflicts, approve threads and set autonomy from the app ([e7bf062](https://github.com/iExalt/acyclic/commit/e7bf0623ab056afae9b98be761564ff24cc5cca0))
* **app:** list what needs the user and notify their devices ([3c0a1be](https://github.com/iExalt/acyclic/commit/3c0a1be998f943511d30bfb8bcde26547578d174))
* **app:** start the app, served by the server built in and installable ([#17](https://github.com/iExalt/acyclic/issues/17)) ([e5407d5](https://github.com/iExalt/acyclic/commit/e5407d59b6c5291866fa865fa1dc501148c0a0e3))
* **cli:** print for people at a terminal and JSON when piped ([#10](https://github.com/iExalt/acyclic/issues/10)) ([08d01db](https://github.com/iExalt/acyclic/commit/08d01dbbc77b657d9c75e2dbe50d34282445b72e))
* make threads the unit of work and give the CLI a human side ([#6](https://github.com/iExalt/acyclic/issues/6)) ([cdbc6de](https://github.com/iExalt/acyclic/commit/cdbc6dee9b1b71189cb21bfd2a355ddc3769512f))
* **operator:** start runs with signed tokens and apply their writes once, as the run ([#13](https://github.com/iExalt/acyclic/issues/13)) ([f80a7a3](https://github.com/iExalt/acyclic/commit/f80a7a39fdb445d77f670173fb948f4b81bd8c2f))
* **secrets:** run Podman machines from mise and hand containers credentials as Podman secrets ([#11](https://github.com/iExalt/acyclic/issues/11)) ([85fe13c](https://github.com/iExalt/acyclic/commit/85fe13c2be66926f06dd07ca0d9dab2550b48c3c))
* **server:** let runs fetch and push their thread's code through the server, from mirrors of the forge ([#15](https://github.com/iExalt/acyclic/issues/15)) ([47c16f9](https://github.com/iExalt/acyclic/commit/47c16f9472d1701eca2b0162537dca75ff7328d9))
* **server:** serve runs' commands on a runners' listener, bound to their thread, scanned and applied once ([#14](https://github.com/iExalt/acyclic/issues/14)) ([e579069](https://github.com/iExalt/acyclic/commit/e579069ac076fb2ff9441a1736ba724d4ed3fcec))
* **server:** serve the tracker on the Mac with acyclic up, a session and an opt-in Tailscale socket ([#12](https://github.com/iExalt/acyclic/issues/12)) ([9b9690e](https://github.com/iExalt/acyclic/commit/9b9690e46fc870c0f6a311819be204dad235b914))


### Bug Fixes

* **app:** close the menu alone, and badge thread numbers once ([#25](https://github.com/iExalt/acyclic/issues/25)) ([71f8938](https://github.com/iExalt/acyclic/commit/71f8938491cd4cf2426c45b6e3f2d9f5875501da))
* **app:** let the next tap through after a menu action, and badge only named threads ([#26](https://github.com/iExalt/acyclic/issues/26)) ([3259e82](https://github.com/iExalt/acyclic/commit/3259e822ece5f127b13496e43855b8ca20bdc81f))
* **cli:** print readiness in pretty output as a phrase ([#9](https://github.com/iExalt/acyclic/issues/9)) ([22d514c](https://github.com/iExalt/acyclic/commit/22d514cde3bb485a58c2e139a2bf80265562a156))
* **server:** share recent mirror fetches, bound a push's refs, and look repositories up within the run's thread ([#16](https://github.com/iExalt/acyclic/issues/16)) ([28e0d2a](https://github.com/iExalt/acyclic/commit/28e0d2a64106cfdd115d98c7196e7a942ed67064))


### Performance Improvements

* **cli:** build each thread's row once in the needs-you list ([f6de37d](https://github.com/iExalt/acyclic/commit/f6de37d361a7f3cff78bf7c5a2643207393508ae))

## [0.2.0](https://github.com/iExalt/acyclic/compare/v0.1.0...v0.2.0) (2026-10-16)


### ⚠ BREAKING CHANGES

* **cli:** a `needs` proposal entry gains `item`.
* **cli:** `needs` entries are tagged `kind`, not `entry`, and embed rows: item ids become `item` rows, thread ids `thread` rows, a proposal entry becomes `{kind, proposal}` with its edit, and `holders` of a resource clash become `{item, holder}`; a conflict's own `kind` is `entity` and it gains `values` and `resolve`; awaiting-approval moves its open member count to `counts.open_members`. `ready` rows drop `rank`; read `readiness.rank`.
* **cli:** renamed keys. A record's own id is `id`, not `item`, `thread` or `repository`; `list` rows gain `state` and `readiness`; `show` drops the nested `item` and flattens `done_at`, `evidence`, `archived` and `reopened` (now booleans), adds `links`, and names claim times `lease_started_at` and `lease_expires_at`; thread `members`, `conflicted` and `done` move into `counts` and `members` in `show` is a list of item rows with `approved` and `conflicted` in place of `in_approval` and `conflict`; repo and proposal `show` no longer nest their record or edit, and an edit is `{kind, field, before, after}`; `inherited` is now `placed` and always present; `import`'s `report` becomes `counts` and `changes`; `doctor` and `graph --output` counts move under `counts` and `unreadable` entries name `entity`; `level` lists threads as an array; error details `differs` is `fields`, `path` is `items` and `until` is `eligible_at`; `proposal keep` returns `item`, `thread` and `approved`.
* **cli:** `item move` and `item state` become `item edit --parent` and `--state`; `thread set`, `thread add <ITEM> <THREAD>` and `thread remove` become `thread add`, `thread edit` and `item edit --thread`; `repo set`, `repo attach` and `repo detach` become `repo add`, `repo edit` and the `--repository` flags; `level --thread` becomes `thread edit --level`. Commands name the subject `item` rather than `id` in the server's JSON. `item add` and `item edit` report `{item, changed, proposals, inherited}` rather than an edit and its outcome, and an add of an existing item with other values is refused instead of changing it.
* **cli:** link kinds on the command line, in the server's JSON and in `link` output are `waits-for`, `follows` and `relates-to`. Import documents written by this build are version 2.

### Features

* **cli:** embed what each needs entry is about ([14fac7b](https://github.com/iExalt/acyclic/commit/14fac7b67ab1d6b165f7ee358d45a1b1565d9a5b))
* **cli:** give the read and write commands' JSON one shape ([51601b0](https://github.com/iExalt/acyclic/commit/51601b08d2fe703cdace69f0cc3214af32c61cc3))
* **cli:** list enum values in help, add the help command and group global options ([da35bad](https://github.com/iExalt/acyclic/commit/da35bad7e487916fa31ae5e57cd8fc173d8018b5))
* **cli:** list every proposal with proposal list ([8f1eebe](https://github.com/iExalt/acyclic/commit/8f1eebeaa48c587c2eee05f32ec98c02ff1d18a8))
* **cli:** name link kinds waits-for, follows and relates-to, and add import format 2 ([6224982](https://github.com/iExalt/acyclic/commit/62249824f3b46025faeeb018f8d235e8575c5ba9))
* **cli:** take every input as a field flag, so add and edit share their fields ([21ab3ad](https://github.com/iExalt/acyclic/commit/21ab3ade11a2544c8953faa80950b9540353d662))
* **core:** delete an item with a tombstone, and `item delete` ([6b5c79a](https://github.com/iExalt/acyclic/commit/6b5c79acab9a37cad4eca6b41cc721a0d4a93dbb))
* **core:** extend an approval to members applied to its thread ([119656e](https://github.com/iExalt/acyclic/commit/119656e273c51abe23bc0c53244f22ebea07bb46))
* **core:** price usage from a versioned table, exactly, when reading ([a9259ee](https://github.com/iExalt/acyclic/commit/a9259ee90f1d3a20d5a834377b6f3da695efa439))
* **core:** record usage per writer and month, and read it once per call ([7e9de49](https://github.com/iExalt/acyclic/commit/7e9de491fc3737efde25ce0ca3f81a613e06004f))
* **core:** report each field an import changes ([cdd2b57](https://github.com/iExalt/acyclic/commit/cdd2b575f3c21eb6babbef5a68a21186ee4d07c9))
* **core:** say in one line why an archive, delete or import is refused ([584fca4](https://github.com/iExalt/acyclic/commit/584fca4b830b85cfedbf98aa2798e93bf364e38b))


### Bug Fixes

* **cli:** describe the item group fully and name the form that is gone ([2ce77d1](https://github.com/iExalt/acyclic/commit/2ce77d14c5211b679ff8d0f0199bbba6ec6691ea))
* **cli:** list an import's placement proposals in proposals ([b4c6da0](https://github.com/iExalt/acyclic/commit/b4c6da0c37ffca24d6d1dc5f1b4736dabf09d457))
* **cli:** list every proposal a write leaves, and show the item of a proposal need ([88933f6](https://github.com/iExalt/acyclic/commit/88933f69653ad089fb2127d482c63e7d4110c5bf))
* **cli:** name the command that changes what an add refused to overwrite ([d5e89e7](https://github.com/iExalt/acyclic/commit/d5e89e795e0a624b50b74338ec6b9f26402a067a))
* **cli:** name the list command in not-found errors and the rules in item delete help ([9f2f92e](https://github.com/iExalt/acyclic/commit/9f2f92ebd560c0f6a38962af2aaac4f12289e8c0))
* **cli:** refuse a lease outside 1 to 480 minutes, and a repository record created without every field ([9c1f77b](https://github.com/iExalt/acyclic/commit/9c1f77b159a6ab360a68c641ac58627d02386e67))
* **cli:** refuse done evidence that can never verify, and evidence that replaces other evidence ([93033a3](https://github.com/iExalt/acyclic/commit/93033a3fb78557a4f696679f450d2c9bd7b38688))
* **cli:** say a lease takes 1 to 480 minutes, and tell agents how to set up a clone ([d24540a](https://github.com/iExalt/acyclic/commit/d24540ad32b458bb55a88aae09ff4cf4caf82e19))
* **cli:** say which commands print text, mark user-only help, and name the fix in errors ([c8354f5](https://github.com/iExalt/acyclic/commit/c8354f5501b4a47a18bdcd41eb10cb0f711db4a5))
* **cli:** stop quietly when the reader of standard output has gone ([1b2039d](https://github.com/iExalt/acyclic/commit/1b2039d07cc255e4ea215ec85111d68dfc9b9913))
* **core:** leave a call unpriced unless the table can price all of it ([dafa8a3](https://github.com/iExalt/acyclic/commit/dafa8a3fd595cf447b8953889173e450dc4f02dc))
* **core:** name the command that fixes a store, locate or import error ([0a918a6](https://github.com/iExalt/acyclic/commit/0a918a6c204be03da36f099d683ab018352f76f8))
* **core:** name the item a refused archive or delete is about ([28a96a9](https://github.com/iExalt/acyclic/commit/28a96a9a19aba913f60cb97cde54f93723395113))
