# Changelog

## [0.7.2](https://github.com/iExalt/acyclic/compare/v0.7.1...v0.7.2) (2026-11-06)


### Bug Fixes

* **app:** remove Map dimming during hover and selection ([#217](https://github.com/iExalt/acyclic/issues/217)) ([504505d](https://github.com/iExalt/acyclic/commit/504505d2342d8a9a80ad5074a727feef84e613c9))

## [0.7.1](https://github.com/iExalt/acyclic/compare/v0.7.0...v0.7.1) (2026-11-06)


### Bug Fixes

* **app:** align Map state icons on the right ([#209](https://github.com/iExalt/acyclic/issues/209)) ([9944ed6](https://github.com/iExalt/acyclic/commit/9944ed64784027b1dd6a63c8325b6f425bf0a876))
* **app:** anchor epic resize animation and remove content fades ([#215](https://github.com/iExalt/acyclic/issues/215)) ([dc103f9](https://github.com/iExalt/acyclic/commit/dc103f9d23de189a08b62be8a5d3c69797ca681d))
* **app:** fold completed items in unfinished threads ([#210](https://github.com/iExalt/acyclic/issues/210)) ([c2fba27](https://github.com/iExalt/acyclic/commit/c2fba27b2768ce23b77f5a00c73000d03cecbd7b))
* **app:** keep connections steady over disconnected Map items ([#207](https://github.com/iExalt/acyclic/issues/207)) ([af1094e](https://github.com/iExalt/acyclic/commit/af1094e3b5b1cbd98b99254510f22b8db0dcdf53))
* **app:** standardize Map item sizes and truncate long titles ([#211](https://github.com/iExalt/acyclic/issues/211)) ([8b6de40](https://github.com/iExalt/acyclic/commit/8b6de4037d2f9bdddb2b131e405694fa43bf66cf))


### Performance Improvements

* build releases without LTO, keeping one codegen unit ([#214](https://github.com/iExalt/acyclic/issues/214)) ([1ed625e](https://github.com/iExalt/acyclic/commit/1ed625e0b0c6f8a19e33186ff1e340a3b5ced8e7))

## [0.7.0](https://github.com/iExalt/acyclic/compare/v0.6.0...v0.7.0) (2026-11-06)


### ⚠ BREAKING CHANGES

* adding, renaming, moving, reopening or un-canceling an item can now be refused when its title is taken in its initiative. Item candidates in `ambiguous` errors gain `initiative`. Needs you gains the `duplicate-title` kind.
* `item add` needs a parent. `--no-parent` is removed. Import accepts only version 4. `role` loses `group`. The dangling-parent Needs-you kind becomes `parent-missing`. `thread ready` is a struct variant on the wire (`{"thread":{"ready":{}}}`). The audit log moves to per-initiative files.
* the `unthreaded` readiness, stage, count and Needs-you kind are gone. Thread rows gain `task`, and thread ids may now end in `.one`. A leaf in no thread is ready and claimable.
* a lapsed claim on ready work reads `stalled`, not `ready`. JSON gains the stage `stalled`, the counts key `stalled`, the thread readiness `stalled`, the `stalled` flag on `thread ready` rows, and the Needs-you kind `stalled`.
* a parent's stored thread no longer counts anywhere. New items no longer fall back to an ancestor's thread or skip done siblings. `item add` can now refuse with `thread-unclear`, and Needs you has two new kinds: `thread-on-parent` and `empty-thread`.
* `canceled` is the first stored state older replicas cannot read. Upgrade every replica before canceling anything. New fields: the state value `canceled`, the stage `canceled`, the counts key `canceled`, the wait reason `canceled`, and the Needs-you kind `waits-on-canceled`.
* Thread rows in JSON no longer have `heads` or `counts`. `thread list` shows `done` and `leaves` columns. Needs entries gain `summary`.
* `thread ready` orders threads and items by id rather than by title, and Up next is ordered by id rather than by wave.
* thread JSON has `notes` where it had `brief`, and `thread add` and `thread edit` take `--notes` and `--no-notes` (the old flags and the import key `brief` are still accepted). Readiness JSON is tagged `readiness` instead of `status`. Needs, doctor and import-report entries name a `record` instead of an `entity`. The `link-cycle` need kind and error code are now `waits-for-cycle`. `progress` is `{counts}` only, so `progress.state` is gone.
* Readiness JSON's `blocked` tag is now `waiting`, and the stage counts key `blocked` is now `waiting`. Item state shows as `open` where it said `backlog`. `--state` takes `open` or `parked`, and still accepts `backlog`. Imports accept `open` and `backlog`.
* `item add`, `item edit`, `thread add` and `thread edit` no longer accept `-r`, `--repository` or `--no-repository`. Item and thread JSON no longer has `repositories`, and repository JSON no longer has `installation`. `repository show` returns only the repository record, because its item and thread lists came from the removed sets. Imports that carry `repositories` on an item or thread, or `installation` on a repository, are refused, and the error names the key. The needs list no longer reports dangling repositories on items or threads.
* `item add` and `item edit` no longer accept `--size` or `--no-size`. Item JSON no longer has `size`. Imports that carry a `size` key are refused, and the error names the key. The app's edit contract drops the `size` field.
* `item archive` and `thread ready --include-archived` are removed. Imports that carry an `archived` key, or store done on a parent, are refused. Item JSON no longer has `archived` or `reopened`. `item reopen` refuses parents, with error `parent`, and items that aren't done, with `not-done`. A waits-for link between an item and its ancestor or descendant is refused with `lineage-link`.
* readiness loses `in-review` and `unverified`, and a wait's `why` loses them too. Stage loses `in-review`. `progress.counts` loses `in_review`, and `done` counts only done leaves. `needs` no longer lists `unverified`. `item done --commit` and `import` refuse unmerged or unknown commits with code `unmerged`, and `doctor` adds `commits`. Rust callers:
* **core:** Rust callers must use `ready::ReadinessEvaluator` for the former `ready::Readiness` calculator, `ready::Readiness` for `ready::Status`, `ready::ThreadReadiness` for `ready::ThreadStatus`, and `model::Role::Leaf` for `model::Role::Item`. The public CLI `args::StateWord` is a validated newtype rather than an enum and no longer implements `clap::ValueEnum`; construct it through `TryFrom<ItemState>` instead of its former variants. `import::State` remains a compatibility re-export of `model::ItemState`. JSON retains the `status` key and `item` leaf role, and `--state` continues to accept only `backlog` and `parked`.
* preserve scoped writes and report internal import refusals ([#151](https://github.com/iExalt/acyclic/issues/151))
* guard tracker setup and archive lifecycle ([#149](https://github.com/iExalt/acyclic/issues/149))

### Features

* add initiative records as the top level, and require every item to have a parent ([230d77f](https://github.com/iExalt/acyclic/commit/230d77fc556863dc4d05ebd83fd37b5c42c5114d))
* add the canceled state ([47747ce](https://github.com/iExalt/acyclic/commit/47747ce63e0d916313a593f71e7d8427e237815f))
* **app:** add an initiative switcher and group the lists under initiatives ([5f3bf7a](https://github.com/iExalt/acyclic/commit/5f3bf7ad0cb79b440bfde15944d64009ea9630f7))
* **app:** glide the Map's tiles to their new places and wait out a gesture before redrawing ([#197](https://github.com/iExalt/acyclic/issues/197)) ([3b2989e](https://github.com/iExalt/acyclic/commit/3b2989e5257ef91f5896df6d1839657668f4ce0c))
* **app:** scope the Map, Threads view and outline to the initiative in context ([891b990](https://github.com/iExalt/acyclic/commit/891b99025c31f4ff2b44b91dc983ffdfff872818))
* **app:** show work that crosses initiatives as stubs ([266dced](https://github.com/iExalt/acyclic/commit/266dced859825798a02bdae56bcbf0ab941480f0))
* **app:** slide boxes into the room a closed box gives up ([#198](https://github.com/iExalt/acyclic/issues/198)) ([6a7a207](https://github.com/iExalt/acyclic/commit/6a7a2076b92234cc8d821d0d74abeec942f9fb2b))
* check commit evidence when it is written, not on every read ([db5d246](https://github.com/iExalt/acyclic/commit/db5d246de034ecfbd28a07c4c12bac7b118e69c5))
* **core:** run the store on gix, with no git executable ([#165](https://github.com/iExalt/acyclic/issues/165)) ([aefc921](https://github.com/iExalt/acyclic/commit/aefc921dc12c732cb42773f2f00d521e0da57cbc))
* derive a parent's done from its leaves and retire archiving ([44e38aa](https://github.com/iExalt/acyclic/commit/44e38aa4931457d3afd901a96e18529ea599ea51))
* make titles unique among unfinished items within an initiative ([596e127](https://github.com/iExalt/acyclic/commit/596e127ad7fe81f79894f0d14d49a1f6ec6f7070))
* new children inherit their siblings' thread and moves never change one ([c4b5eff](https://github.com/iExalt/acyclic/commit/c4b5effc43e519092db36c9ce468c66f474425da))
* order ready work by id ([36400f4](https://github.com/iExalt/acyclic/commit/36400f47c268a1b3c4b96be616c52ed5972a3261))
* read a task in no thread as a thread of one, so it is ready and claimed like any thread ([646dc9e](https://github.com/iExalt/acyclic/commit/646dc9e98c3dad816f8f4ced3b712e49689926ae))
* read a thread whose lease ran out as stalled, and renew leases after a batch's writes ([0d57a5b](https://github.com/iExalt/acyclic/commit/0d57a5bf4ffc2bcd50fe55be0907bb520699ad0d))
* retire item and thread repository sets and repository installation ([419e109](https://github.com/iExalt/acyclic/commit/419e109c44e5a258e817ba556386228e038fbc5c))
* retire the item size field and let retired fields lie dormant ([2ca7727](https://github.com/iExalt/acyclic/commit/2ca7727ef027ced0b89caa74213c447919dc4447))
* retire thread heads and word Needs you once in core ([ac8e55d](https://github.com/iExalt/acyclic/commit/ac8e55da245271b7f3d30f19d7f88c3365d6def1))
* say notes, readiness and record, and retire a parent's progress state ([e78f666](https://github.com/iExalt/acyclic/commit/e78f6663aec78724ec4d1c23766d4ec1b6440bbb))
* say waiting for blocked work and open for backlog ([dcca254](https://github.com/iExalt/acyclic/commit/dcca254f5797935a53ee90ac0f2fa4288fe9f89f))
* show a task in no thread as a task alone in the app, and quiet the stalled look ([fe6a611](https://github.com/iExalt/acyclic/commit/fe6a6114bc74573ceec7b597cb70284d4449699b))


### Bug Fixes

* **app:** clarify task labels and fit full Map titles ([0fba049](https://github.com/iExalt/acyclic/commit/0fba04921cf1324bf3de50b1a1fe66be28813813))
* **app:** keep search results visible and preserve map pins ([#160](https://github.com/iExalt/acyclic/issues/160)) ([c1b313c](https://github.com/iExalt/acyclic/commit/c1b313c7ef11632ba22f197ac85d8cac487bb5fe))
* **app:** preserve navigation history and prioritize exact search matches ([#159](https://github.com/iExalt/acyclic/issues/159)) ([99a1cdc](https://github.com/iExalt/acyclic/commit/99a1cdcbf9cd64d7a78ac20cefd1aea8d00ea103))
* **app:** refine phone forms, notices, and touch interactions ([37c1e38](https://github.com/iExalt/acyclic/commit/37c1e38df1251f5325a98b3fd000f8166098440f))
* **app:** say when the Map has settled, and wait on it in the journeys ([#191](https://github.com/iExalt/acyclic/issues/191)) ([28810a7](https://github.com/iExalt/acyclic/commit/28810a72e1fda0970b3d5c13a5880d75e843a33e))
* **app:** stabilize map packing, framing, and thread order ([1abe43a](https://github.com/iExalt/acyclic/commit/1abe43af2cf7cb99a03e4928de8c30c4990e6cb9))
* clarify claim renewal and safe clock recovery ([#153](https://github.com/iExalt/acyclic/issues/153)) ([a70d00b](https://github.com/iExalt/acyclic/commit/a70d00b42b5a6a2b5689610f8867cfcbd9632d72))
* describe import updates and tracker sync results ([#156](https://github.com/iExalt/acyclic/issues/156)) ([f2b8ac4](https://github.com/iExalt/acyclic/commit/f2b8ac4ec48843e6f8dc54d37e87c25e68fa2040))
* explain the six-character minimum when an id ending is too short ([#179](https://github.com/iExalt/acyclic/issues/179)) ([10ae37b](https://github.com/iExalt/acyclic/commit/10ae37b223d9e43219a5c81f188ac0b28a2e4c69))
* give crates made from the template kebab-case package names ([#182](https://github.com/iExalt/acyclic/issues/182)) ([b6a0db0](https://github.com/iExalt/acyclic/commit/b6a0db0162eaf6da7b35f471b12812604a8001f6))
* guard app undo, conflict choices, and need acknowledgments ([703cf8a](https://github.com/iExalt/acyclic/commit/703cf8a4cff4fd052a1d24ee42ddcb64dfcd8a8e))
* guard tracker setup and archive lifecycle ([#149](https://github.com/iExalt/acyclic/issues/149)) ([9d3e4cd](https://github.com/iExalt/acyclic/commit/9d3e4cd3a5cd0e25dcbae054a7f609c0643cb6f9))
* hold only the pressed remove button and leave no hover tint after a tap ([#187](https://github.com/iExalt/acyclic/issues/187)) ([bb1cd54](https://github.com/iExalt/acyclic/commit/bb1cd545cd9f03bc9f2660a662e733dbf5783575))
* keep fetches from prompting on a terminal ([#180](https://github.com/iExalt/acyclic/issues/180)) ([709af2a](https://github.com/iExalt/acyclic/commit/709af2a80a9d114cb7af6cbc8a73d084ef261cf9))
* keep item status and error explanations consistent ([#157](https://github.com/iExalt/acyclic/issues/157)) ([9a841ed](https://github.com/iExalt/acyclic/commit/9a841ed6374d5db590f9ad9fd0271ab9a1f70c32))
* keep the fixture generator's threads from forming cycles and shape small fixtures like large ones ([#190](https://github.com/iExalt/acyclic/issues/190)) ([9accd8d](https://github.com/iExalt/acyclic/commit/9accd8d0fac9f823ad35eda1d48be8aae10b6164))
* let pick mode take typing to find the target and round the Needs you focus ring ([#185](https://github.com/iExalt/acyclic/issues/185)) ([48f92c2](https://github.com/iExalt/acyclic/commit/48f92c2af98e426a0dc386cba6d3036f7e023906))
* preserve scoped writes and report internal import refusals ([#151](https://github.com/iExalt/acyclic/issues/151)) ([62cc12b](https://github.com/iExalt/acyclic/commit/62cc12bd7a0e7272e94b6106689a9cf8d11cde1d))
* report write refusals and conflict choices accurately ([#155](https://github.com/iExalt/acyclic/issues/155)) ([3b3eb25](https://github.com/iExalt/acyclic/commit/3b3eb25a8c3fe17fb03d51b714294d016584d073))
* restore actionable tracker setup and diagnostics ([#152](https://github.com/iExalt/acyclic/issues/152)) ([e0fafdb](https://github.com/iExalt/acyclic/commit/e0fafdb3a60a15306842c8a13f7db9ac8619dc0b))
* stop a GET from creating the push key ([#178](https://github.com/iExalt/acyclic/issues/178)) ([a9cb699](https://github.com/iExalt/acyclic/commit/a9cb699959852d5d3257a91bce4811e48dd17c92))
* wrap long code in notes, fill a project's whole row on hover, and line up the New item header with the item panel's ([#188](https://github.com/iExalt/acyclic/issues/188)) ([1f89e9c](https://github.com/iExalt/acyclic/commit/1f89e9ce168d12bdb78791d1b8ede992998207fd))


### Performance Improvements

* **core:** touch a pack once when a sync rewrites what it fetched ([38ad06e](https://github.com/iExalt/acyclic/commit/38ad06e72f7f930a9ff50b1668e2cba61cb5d30f))
* skip the whole-tracker cycle and lineage walks when a change set writes no parent or link ([#194](https://github.com/iExalt/acyclic/issues/194)) ([cb38ef5](https://github.com/iExalt/acyclic/commit/cb38ef568cc3bdc19ae815cb1dc7fea913dd5711))


### Code Refactoring

* **core:** align tracker terminology ([d510ed2](https://github.com/iExalt/acyclic/commit/d510ed2a9d74fdd40deea8a5a1a870f59fdfe887))

## [0.6.0](https://github.com/iExalt/acyclic/compare/v0.5.2...v0.6.0) (2026-11-03)


### ⚠ BREAKING CHANGES

* **up:** `acyclic up --link` and `--rotate` are gone, and `up` no longer prints `session` or `link`. A Tailscale setup pointed at the old secret path must run the printed `tailscale serve` command again.
* a conflict entry no longer has `resolve`; each value's `run` replaces it.
* **core:** the `checked-field` error code is gone. When `item done` breaks several rules at once, the one reported follows the judge's order (usage, then not found, then refused), so an invalid commit on work that isn't ready now reports `invalid-commit` rather than `not-ready`.
* **cli:** judge item, thread and repository adds and edits as change sets ([#135](https://github.com/iExalt/acyclic/issues/135))

### Features

* **app:** draw each parent's stage and each thread's counts as core rolls them up ([#145](https://github.com/iExalt/acyclic/issues/145)) ([890bb1b](https://github.com/iExalt/acyclic/commit/890bb1b6285a8d4c2b33f75b58d1a03eb83e221d))
* **app:** fold each Map box's finished work into done boxes ([#143](https://github.com/iExalt/acyclic/issues/143)) ([ae06d6c](https://github.com/iExalt/acyclic/commit/ae06d6c31176b837e7a9d8631523ca16559cbe26))
* **app:** restore where an item or thread was opened from on Forward, and frame a thread from its item ([#141](https://github.com/iExalt/acyclic/issues/141)) ([9dc81ad](https://github.com/iExalt/acyclic/commit/9dc81ad488a796b22897ee0c52fb9ab1e0cbee9f))
* **cli:** judge item, thread and repository adds and edits as change sets ([#135](https://github.com/iExalt/acyclic/issues/135)) ([031de16](https://github.com/iExalt/acyclic/commit/031de16c1eb594fdcb0673ec6fb7c4cf3de49f08))
* **core:** judge finishing work and claims, and keep record writes behind the judge ([#136](https://github.com/iExalt/acyclic/issues/136)) ([d7244c7](https://github.com/iExalt/acyclic/commit/d7244c73fe3700b1af9745634d3c724d92714527))
* **core:** judge imports as one change set in a single write module ([#134](https://github.com/iExalt/acyclic/issues/134)) ([1d50274](https://github.com/iExalt/acyclic/commit/1d50274fed34d360410a11f75ca3d9b0cf969953))
* **core:** key needs-you entries in core and say how each conflicted value is kept ([#138](https://github.com/iExalt/acyclic/issues/138)) ([67f49f2](https://github.com/iExalt/acyclic/commit/67f49f2f2edf9742a990d4e40105e6b6573b118b))
* **core:** roll a parent's stage up from its leaves and count each thread's leaves by stage ([#142](https://github.com/iExalt/acyclic/issues/142)) ([dc21706](https://github.com/iExalt/acyclic/commit/dc217067120c93f7cf96857bdf3bf2634aeae35b))
* offer the command that keeps each value of a conflict, from one renderer ([#139](https://github.com/iExalt/acyclic/issues/139)) ([ab1e1d4](https://github.com/iExalt/acyclic/commit/ab1e1d4bc22dcca674ba442f2207a0d532484f6b))
* **up:** drop pairing and the session; trust local processes ([#147](https://github.com/iExalt/acyclic/issues/147)) ([dcf3f28](https://github.com/iExalt/acyclic/commit/dcf3f280820e41a0b39ad4f9c252d2f39630b778))


### Bug Fixes

* **app:** hold the Map's top level from its last packing, so opening and closing order doesn't move boxes ([#148](https://github.com/iExalt/acyclic/issues/148)) ([f71d67d](https://github.com/iExalt/acyclic/commit/f71d67d66d8b7541ed5815588985ef181a10e517))
* **app:** keep the Map's top-level boxes in place when a box inside one opens ([#146](https://github.com/iExalt/acyclic/issues/146)) ([d20df25](https://github.com/iExalt/acyclic/commit/d20df250e74b406cdcdbec9450354e826b2f53f9))
* **core:** refuse a write that settles a parent or link on a value closing a cycle ([#137](https://github.com/iExalt/acyclic/issues/137)) ([485ac36](https://github.com/iExalt/acyclic/commit/485ac366ec11e575c2af0bb0cf473f74702c8ecd))
* count a thread only while it has a record, and list an item in a missing thread once on needs-you ([#133](https://github.com/iExalt/acyclic/issues/133)) ([0adeacc](https://github.com/iExalt/acyclic/commit/0adeacc1a98b2819df351768d3a3b3bd70c9d0e4))
* **server:** key needs-you notifications by what each reference points at ([#131](https://github.com/iExalt/acyclic/issues/131)) ([c6cf185](https://github.com/iExalt/acyclic/commit/c6cf185f53fe354d38309beb36afcf494f286d1b))

## [0.5.2](https://github.com/iExalt/acyclic/compare/v0.5.1...v0.5.2) (2026-11-02)


### Bug Fixes

* **core:** fail to read a state whose tree names a missing object ([#126](https://github.com/iExalt/acyclic/issues/126)) ([b93bf85](https://github.com/iExalt/acyclic/commit/b93bf858fdc862340fe002a8aa86525f54273e35))
* give a draft refused as deleted a new id, and stop pointing at a migration ([#129](https://github.com/iExalt/acyclic/issues/129)) ([9073d93](https://github.com/iExalt/acyclic/commit/9073d93ab6cb51750f9d94cadda1f9378a5f1e12))
* reopen work set back from done, refuse deleted threads on import, and keep printed endings apart from titles ([#128](https://github.com/iExalt/acyclic/issues/128)) ([0fd0f03](https://github.com/iExalt/acyclic/commit/0fd0f03dc1c639257305b3b292b0596f7222175e))


### Performance Improvements

* **cli:** read the model once per item or thread write ([#130](https://github.com/iExalt/acyclic/issues/130)) ([891ec1e](https://github.com/iExalt/acyclic/commit/891ec1e9736f5ada35116238431926a4c2409980))

## [0.5.1](https://github.com/iExalt/acyclic/compare/v0.5.0...v0.5.1) (2026-11-02)


### Bug Fixes

* bring docs, messages and comments in line with what the code does ([#121](https://github.com/iExalt/acyclic/issues/121)) ([5344eb2](https://github.com/iExalt/acyclic/commit/5344eb2eed7f197a102864d88ec2dac40ac58e94))
* keep conflicted evidence in the evidence shape, key each dangling need by its target, and hint at what reads an old tracker ([#123](https://github.com/iExalt/acyclic/issues/123)) ([909b1cd](https://github.com/iExalt/acyclic/commit/909b1cdc0783aff9afb71df32671fd805d1f8cfc))
* refuse clashing repository names on import and add-and-remove of one record ([#122](https://github.com/iExalt/acyclic/issues/122)) ([f94febc](https://github.com/iExalt/acyclic/commit/f94febcfe68117fae76acfe59df561730d7c3245))


### Performance Improvements

* **cli:** name records in pretty output from the state the command read or wrote ([#124](https://github.com/iExalt/acyclic/issues/124)) ([3f4f9c0](https://github.com/iExalt/acyclic/commit/3f4f9c0b56fd0cfb576e8c5930eabd31e9faa7ff))


### Reverts

* **core:** run git from PATH again instead of resolving macOS's xcrun stub ([#119](https://github.com/iExalt/acyclic/issues/119)) ([a82bd3c](https://github.com/iExalt/acyclic/commit/a82bd3c9eb654e93cabe68e67153782a35ba5004))

## [0.5.0](https://github.com/iExalt/acyclic/compare/v0.4.0...v0.5.0) (2026-11-02)


### ⚠ BREAKING CHANGES

* record ids are UUIDv7s, and references resolve by full id, exact title or `owner/name`, or an id ending of at least 6 characters, refusing an ambiguous one with the new `ambiguous` error; an invalid `--id` or imported id is the new `invalid-id` error, and `reserved-id` is gone. `item add` and `thread add` take `--id` instead of a positional id, and `repository add` takes `--name <OWNER/NAME>` and an optional `--id` instead of a positional id and `--owner`; `repository edit` loses `--owner`. `item done` takes `--commit <OWNER/NAME>@<SHA>` and loses `--repository`. Evidence's `note` is now `text`, and the evidence view is `{commit: "owner/name@sha", text}`. Import reads only version 3, with `id` in place of `key`, an item's `thread` in place of a thread's `items`, and evidence as `{commit: "<repository id>@<sha>", text}`. The state format version is bumped, so a tracker written by an earlier version is refused until migrated. `clones.toml` is keyed by `owner/name`, not by repository id. `thread renumber` is removed. In JSON, adds take an `id` key, the positional `item`, `thread` and `repository` keys on adds are dropped, and `sync` no longer reports `number_clashes`.
* the `link` command group (`link add`, `link remove`) is removed; use `item add --waits-for` and `item edit --waits-for` / `--no-waits-for`. The flags `--area`/`--no-area`, `--resource`/`--no-resource` and `--gate`/`--no-gate` are removed from `item add` and `item edit`, and `item edit --state` no longer takes `in-progress`. Item JSON drops `gate`, `areas`, `resources` and `links.relates_to`/`links.related_from`. The `resource-busy` readiness, the `resource-held` error and the `resource-clash` need are gone, and a dangling waits-for need now names the waiting item (`entity: item`, `field: waits-for`) and carries a `fix`. Import documents put `waits_for` on items: a `links` list, `areas`, `resources`, `gate` or an `in-progress` state is refused. A tracker holding relates or after links or stored in-progress states can't be read until it is migrated.
* these commands moved, with no aliases, so the old names are now usage errors: `show` is now `item show`, `list` is now `item list`, `done` is now `item done`, `ready` is now `thread ready`, `claim` is now `thread claim` and `release` is now `thread release`. The `repo` group is now `repository` (`repository add`, `edit`, `show` and `list`). `done --note <NOTE>` / `-n` is now `item done --evidence <TEXT>` / `-e`. `--repository` gains the short form `-r` on `item add`, `item edit`, `thread add` and `thread edit`. In the server's API, `"list"` is now `{"item": "list"}`. `{"show": …}` is now `{"item": {"show": …}}`. `{"done": {…, "note": …}}` is now `{"item": {"done": {…, "evidence": …}}}`. `ready`, `claim` and `release` now sit under `thread`, and `repo` is now `repository`.
* `--as-user`, `acyclic level`, the `proposal` commands, `thread approve`/`revoke`, `ready --approved` and `thread add/edit --level/--no-level` are removed. With no actor given, commands write as `actor/user` instead of `agent/undeclared`. The `user-only`, `user-from-environment`, `not-allowed` and `decided` error codes are gone. Thread JSON drops `approved` and `level`. Write and import results drop `proposals`, `placed` entries drop `outcome` and `proposal`, and the import format no longer accepts a thread's `approved`. `needs` no longer lists `awaiting-approval`, `proposal`, `unapproved`, `new-blocker` or `question` entries, and lists thread cycles as `thread-cycle`. The `up` session no longer refuses files that other users can open. `acyclic done` on work that isn't ready or that someone else holds now needs `--force`, for the user too.
* `acyclic graph` and `ACYCLIC_OUTPUT` are removed, import format version 1 and `follows` links are refused, stored `after` links are skipped, and a browser holding only the shared `acyclic-up` cookie must pair again.
* remove agent runs

### Features

* give every record a UUIDv7 id, and match records by id or name ([5d603f3](https://github.com/iExalt/acyclic/commit/5d603f3d7f86c7d9b12811ac160a05e839c06f52))


### Bug Fixes

* **bench:** pick batch items from the ready threads' rows ([0ced095](https://github.com/iExalt/acyclic/commit/0ced0959a1f29a66af406ee408f1ea11f7365bd1))
* **core:** open a tracker without fully parsing every peer's state ([4772d2f](https://github.com/iExalt/acyclic/commit/4772d2f7a4d95dd0ebc1c50a64de2076f104b449))


### Performance Improvements

* run git without the xcrun stub on macOS, and split the two-machine sync test ([438d999](https://github.com/iExalt/acyclic/commit/438d99941407381b41677ee1b139c2ccefdb0119))


### Code Refactoring

* drop areas, resources, gates, relates-to links and the stored in-progress state ([107ef3c](https://github.com/iExalt/acyclic/commit/107ef3c06eaa543c7f70b41b5f3e90dfb8438f4d))
* drop user-only gates, proposals and thread approval ([1571eda](https://github.com/iExalt/acyclic/commit/1571eda5844bb6d8caeabdf2e691c3b309e0d726))
* put every command in its group, and give each flag one meaning ([4f0a162](https://github.com/iExalt/acyclic/commit/4f0a16295f595497c127df1655ddfcd7fc9069be))
* remove agent runs ([9a86330](https://github.com/iExalt/acyclic/commit/9a86330db74376c3367c3e88ef12f3eeff653cd6))
* remove the graph command, migration hints and backward compatibility ([33bc3ac](https://github.com/iExalt/acyclic/commit/33bc3ac72264a1c3268fe8123fac6f69aff80247))

## [0.4.0](https://github.com/iExalt/acyclic/compare/v0.3.0...v0.4.0) (2026-11-02)


### ⚠ BREAKING CHANGES

* **server:** serve acyclic up through Tailscale's Mac apps, on a loopback port under a secret path ([#67](https://github.com/iExalt/acyclic/issues/67))

### Features

* **app:** change settings at once, list a thread's items in order, and add a System, Light or Dark theme ([#100](https://github.com/iExalt/acyclic/issues/100)) ([49b9b51](https://github.com/iExalt/acyclic/commit/49b9b5151514ef03624cb4f7922b1675d67f364c))
* **app:** colour threads' lines in Okhsl, every hue as colourful as its lightness allows ([#93](https://github.com/iExalt/acyclic/issues/93)) ([aa1adc6](https://github.com/iExalt/acyclic/commit/aa1adc67fe567a8c861ffdf0a374a428a5c15d3e))
* **app:** give Needs you its own view beside the Overview, the Map and Threads ([#101](https://github.com/iExalt/acyclic/issues/101)) ([64c40b2](https://github.com/iExalt/acyclic/commit/64c40b21b45c7bb8538d0abb2c91f0bfcd62bbb0))
* **app:** show every notice in the slot, and offer Undo for a first approval while no agent has started its work ([#94](https://github.com/iExalt/acyclic/issues/94)) ([c10f029](https://github.com/iExalt/acyclic/commit/c10f02991ba171e9bbc38a6a3d474a9f53fb1c29))
* **server:** serve acyclic up through Tailscale's Mac apps, on a loopback port under a secret path ([#67](https://github.com/iExalt/acyclic/issues/67)) ([f4a857e](https://github.com/iExalt/acyclic/commit/f4a857e21f86d7b454e15a012294f63bfb8a301f))


### Bug Fixes

* **app:** close an item or a thread back to the view it was opened from, and show a searched thread over Needs you ([#74](https://github.com/iExalt/acyclic/issues/74)) ([fc33736](https://github.com/iExalt/acyclic/commit/fc33736bd3905ba96c59378402aeeebfd889f5e5))
* **app:** explain autonomy and gates, name the outline once, and say links and undos as they read ([#72](https://github.com/iExalt/acyclic/issues/72)) ([7b1fe52](https://github.com/iExalt/acyclic/commit/7b1fe5262d3e01dea030a190687811fa782f6cc9))
* **app:** fold the sidebar to a rail, scroll it as one with a line per row, show eight needs wide, and cut only an Undo to a line in the slot ([#78](https://github.com/iExalt/acyclic/issues/78)) ([84c8142](https://github.com/iExalt/acyclic/commit/84c8142e27806dd436b6d2dd7def2de16e587e5a))
* **app:** frame a thread from its sidebar row while one of its items is open, show a new item at a readable zoom, and list merges one cause leaves unconfirmed as one entry ([#103](https://github.com/iExalt/acyclic/issues/103)) ([dc85406](https://github.com/iExalt/acyclic/commit/dc85406845f640c3e42020154f6a557bcc5c6b01))
* **app:** keep each notice with what it is about, and put Undo and a stale plan in a slot at the stage's foot ([#76](https://github.com/iExalt/acyclic/issues/76)) ([5709742](https://github.com/iExalt/acyclic/commit/570974259475632bfa969285c8d2ef05df8185a1))
* **app:** keep panels where they were opened, fill the Overview's columns, and hold only the pressed button ([95e670e](https://github.com/iExalt/acyclic/commit/95e670e2f27639e85028ff023677e3b8515847fc))
* **app:** keep the outline inside the sidebar, open a long menu where it fits, and draw a retitled tile whole ([#92](https://github.com/iExalt/acyclic/issues/92)) ([e6c4837](https://github.com/iExalt/acyclic/commit/e6c48376a9fbb77740576d6b6bec0d4835da8cff))
* **app:** keep the selection in view as the Map shrinks or turns, frame a thread with its boxes' titles, and ask New item for a project rather than an id ([#81](https://github.com/iExalt/acyclic/issues/81)) ([ac83006](https://github.com/iExalt/acyclic/commit/ac83006ccf501c0fd4e29537fe9c8901946d51da))
* **app:** land a link opened before pairing where it pointed, and tell a browser shut out by --rotate to pair again ([#79](https://github.com/iExalt/acyclic/issues/79)) ([6694394](https://github.com/iExalt/acyclic/commit/669439459a86c67edfc6a72c6b4da93500c489ff))
* **app:** leave the Map where it was moved after a tap, and look again within 80 ms while a test waits ([#75](https://github.com/iExalt/acyclic/issues/75)) ([8db4e61](https://github.com/iExalt/acyclic/commit/8db4e61f879d91919a48ae1c55b3c06005ad686b))
* **app:** let a phone's sheets open full or half and close by a drag, and keep a tapped item in view ([#73](https://github.com/iExalt/acyclic/issues/73)) ([da48291](https://github.com/iExalt/acyclic/commit/da4829158a3c8dbe7160c59333ec32492dd3d9f8))
* **app:** list only threads that can start in Up next, and say each item's state one way ([#69](https://github.com/iExalt/acyclic/issues/69)) ([18d8681](https://github.com/iExalt/acyclic/commit/18d86818edfed63d4b015e5fd79c7164cafb4831))
* **app:** name a conflicted title by its first value, say when a project has no threads, space phone rows evenly, and use the glossary's words ([#88](https://github.com/iExalt/acyclic/issues/88)) ([6ec9a40](https://github.com/iExalt/acyclic/commit/6ec9a40a5200534eb232eb9af1c9c2d175e388cc))
* **app:** name things in Needs you, offer Approve again on a new blocker, word refusals for people, and use the glossary's words ([#80](https://github.com/iExalt/acyclic/issues/80)) ([7f2c56e](https://github.com/iExalt/acyclic/commit/7f2c56ed5ccd78e2ab783cea98dd6b1c7be1cc4d))
* **app:** open a thread picked from Needs you on the Map, give the projects a short Up next's column, and tell in-review tiles from a selection ([#99](https://github.com/iExalt/acyclic/issues/99)) ([8934b97](https://github.com/iExalt/acyclic/commit/8934b97603ec38f5f52c35401387662bf707a638))
* **app:** open a thread picked on the Threads view on the Map, as a thread from anywhere else opens ([#106](https://github.com/iExalt/acyclic/issues/106)) ([b14cc1e](https://github.com/iExalt/acyclic/commit/b14cc1ec5e6b087f148cc04a10358333fd1cbb39))
* **app:** open the Map at 1:1 on a phone, centre an item brought in from elsewhere, and reopen an item over the view it was left on ([1bac8be](https://github.com/iExalt/acyclic/commit/1bac8be1232cee6a00e4f0eda65e38a2e2b0b8de))
* **app:** pack the Map's boxes tight, mark where Any time ends, hold a pinch at the furthest zoom out, and show a new item on the Map ([#102](https://github.com/iExalt/acyclic/issues/102)) ([1cfdfdd](https://github.com/iExalt/acyclic/commit/1cfdfdd4bb1df68ec4f73f95713c20af945234a5))
* **app:** reopen on Forward what Back closed, take a menu-picked thread to the Map, keep the Map still when a box collapses, and tidy single keys ([#82](https://github.com/iExalt/acyclic/issues/82)) ([7ba0437](https://github.com/iExalt/acyclic/commit/7ba0437234d11e068d38b89bc088250f905db795))
* **app:** say what Needs you's entries need in words, keep views and panels where the user put them, show the Map laying out, and zoom a picked item in to read ([151e33a](https://github.com/iExalt/acyclic/commit/151e33aa962cba952fd796c83d92dcee55c788cd))
* **app:** set the Overview in three columns on a wide screen, keep an offered Undo over a reload, and bring the docs in line with the app ([55dcd24](https://github.com/iExalt/acyclic/commit/55dcd2455a51a6ac2c88039518c9973ea3230fa6))
* **app:** set the phone's bars, lists, menu and panels compactly, so the Map keeps most of the screen beside a panel ([#104](https://github.com/iExalt/acyclic/issues/104)) ([7ee2e2a](https://github.com/iExalt/acyclic/commit/7ee2e2aedb2fdb70986a7f101d9bf408cab71334))
* **app:** show a task's title on three lines, open the Map at 1:1 on a wide screen, drop backticks on the canvas, and draw links across boxes under the tiles ([#87](https://github.com/iExalt/acyclic/issues/87)) ([dac4a0d](https://github.com/iExalt/acyclic/commit/dac4a0d4b6f4fa256ca00890ff3a3538fb64aef4))
* **app:** slide a phone's sheet in opaque, and fail the build for an app chunk over 500 kB ([#84](https://github.com/iExalt/acyclic/issues/84)) ([5872857](https://github.com/iExalt/acyclic/commit/5872857c8f373ae3da7304bee3424deee7b92298))


### Performance Improvements

* **core:** look a clone's default branch up once per read, not twice per done dependency ([#89](https://github.com/iExalt/acyclic/issues/89)) ([ee053a7](https://github.com/iExalt/acyclic/commit/ee053a7420b7a124ca95aa0f5d4331d9f6139f18))

## [0.3.0](https://github.com/iExalt/acyclic/compare/v0.2.0...v0.3.0) (2026-10-31)


### ⚠ BREAKING CHANGES

* claims move from items to threads: `claim` and `release` take a thread and return `{thread, …}`, the server's claim body names `thread`, `done` no longer releases, and claims older versions made on items are ignored. `ready` returns threads with their items; readiness drops `rank`, adds `unthreaded`, and a busy resource names its `thread`; needs gains an `unthreaded` entry, its contest, clock-ahead and unknown-run entries carry `thread`, and resource-clash holders are (thread, holder) pairs. Follows links can no longer be added, and links drop `follows` and `followed_by`. `proposals` in write results holds `{id, field}` objects, and `graph --output <FILE>` is now `graph --file <FILE>`. Older binaries ignore a claim on a thread but can't see one either, so don't mix this build with older ones that still claim items.

### Features

* **app:** add, move and link items on the Map, with an inspector ([a04bc50](https://github.com/iExalt/acyclic/commit/a04bc5054e490f70ef045d3d7ac816ee2be63185))
* **app:** draw each task's thread and stage, and count a parent's leaves by stage in words ([#52](https://github.com/iExalt/acyclic/issues/52)) ([ecdaa3b](https://github.com/iExalt/acyclic/commit/ecdaa3bd59e9069ea408beb45a57f4598846076a))
* **app:** draw the Map, laid out box by box, with Cytoscape.js ([2639fe9](https://github.com/iExalt/acyclic/commit/2639fe9b9102d46bce20329ac9a92b874ed2ecbe))
* **app:** draw threads as numbered, coloured lines, in a quieter shell ([8246fbd](https://github.com/iExalt/acyclic/commit/8246fbd33fdb5038fb703e8921757594c58ee552))
* **app:** edit fields, settle conflicts, approve threads and set autonomy from the app ([e7bf062](https://github.com/iExalt/acyclic/commit/e7bf0623ab056afae9b98be761564ff24cc5cca0))
* **app:** lay the Map's top level out in id order, shaped like the Map ([#44](https://github.com/iExalt/acyclic/issues/44)) ([399b710](https://github.com/iExalt/acyclic/commit/399b710e8e62b01c31ec3aba51866f4677a698d6))
* **app:** list phases in the sidebar, each opening into its threads, and mark a selected item's phase and thread ([#53](https://github.com/iExalt/acyclic/issues/53)) ([3bf81b5](https://github.com/iExalt/acyclic/commit/3bf81b504fc3e632374fcb5d9e8028329e84a8d7))
* **app:** list what needs the user and notify their devices ([3c0a1be](https://github.com/iExalt/acyclic/commit/3c0a1be998f943511d30bfb8bcde26547578d174))
* **app:** name the needs-you queue Review, count it to 99+, and filter it by what to do ([#50](https://github.com/iExalt/acyclic/issues/50)) ([41e019f](https://github.com/iExalt/acyclic/commit/41e019fbb22faa9760389ddf79ba255c10248ae5))
* **app:** number threads by their ids, and tell clashing numbers apart by clone ([#48](https://github.com/iExalt/acyclic/issues/48)) ([a153962](https://github.com/iExalt/acyclic/commit/a153962527ca16f33496e0e111341548a9c98444))
* **app:** open boxes by their chevrons, select them by their titles, and cross out archived items ([#35](https://github.com/iExalt/acyclic/issues/35)) ([f254d4e](https://github.com/iExalt/acyclic/commit/f254d4e5471462c7f4f22dd003f59391a77ef081))
* **app:** redesign the app in a quiet look, with an Overview home, a command menu, keyboard shortcuts, links and undo ([909637c](https://github.com/iExalt/acyclic/commit/909637c32748b457092d4257d004863d66ab00d9))
* **app:** route each link inside a box around its other tasks, and draw a link another way implies only when asked ([#55](https://github.com/iExalt/acyclic/issues/55)) ([4643e2e](https://github.com/iExalt/acyclic/commit/4643e2e82f4fd476d0b31e4088a1f5db90938bed))
* **app:** route the Threads view's arrows through the gaps between cards, and let its journey run before the Map's edits ([#61](https://github.com/iExalt/acyclic/issues/61)) ([77b8e16](https://github.com/iExalt/acyclic/commit/77b8e1661038180a69d2e64a5c80ea59a125cd5c))
* **app:** set backticked text as code, and make the project-management flag a two-way choice ([#49](https://github.com/iExalt/acyclic/issues/49)) ([bbe52f7](https://github.com/iExalt/acyclic/commit/bbe52f716d4feb8bf7b34432c4ab7460dc84f701))
* **app:** show a parent's progress in the inspector and on its box ([#42](https://github.com/iExalt/acyclic/issues/42)) ([19f8154](https://github.com/iExalt/acyclic/commit/19f8154bd5cfb1b7e00789dd080e3af31be28c42))
* **app:** show threads in waves of what to start first, beside the Map ([#57](https://github.com/iExalt/acyclic/issues/57)) ([8d41f24](https://github.com/iExalt/acyclic/commit/8d41f24bc50548ac94791bca044ded1adf3d0e8e))
* **app:** spiral thread colours round the hue wheel, and mark the needs-you signal with ! ([#39](https://github.com/iExalt/acyclic/issues/39)) ([4539d78](https://github.com/iExalt/acyclic/commit/4539d780c13c09fa2b081f95eae5b2544b9132d6))
* **app:** stand a box's unlinked tasks in columns beside its graph, and centre a lone box title in its strip ([#54](https://github.com/iExalt/acyclic/issues/54)) ([942604d](https://github.com/iExalt/acyclic/commit/942604d6dd7521aea49a30d2cdafb9afc9443789))
* **app:** start the app, served by the server built in and installable ([#17](https://github.com/iExalt/acyclic/issues/17)) ([e5407d5](https://github.com/iExalt/acyclic/commit/e5407d59b6c5291866fa865fa1dc501148c0a0e3))
* **app:** zoom the Map faster per mouse-wheel notch ([#38](https://github.com/iExalt/acyclic/issues/38)) ([d219a96](https://github.com/iExalt/acyclic/commit/d219a96effa2362c2c89f81f69b9f04233bff47e))
* assign item and thread ids in the tracker when an add leaves them out ([#46](https://github.com/iExalt/acyclic/issues/46)) ([b22a6ee](https://github.com/iExalt/acyclic/commit/b22a6ee23868638141055e4bf2fe17d712a55a51))
* **cli:** name the thread by its title in approve and revoke output ([#45](https://github.com/iExalt/acyclic/issues/45)) ([156eb72](https://github.com/iExalt/acyclic/commit/156eb72a940b9875410c825eb3ed6f1b6621b627))
* **cli:** print for people at a terminal and JSON when piped ([#10](https://github.com/iExalt/acyclic/issues/10)) ([08d01db](https://github.com/iExalt/acyclic/commit/08d01dbbc77b657d9c75e2dbe50d34282445b72e))
* derive a parent's state from the work under it ([#41](https://github.com/iExalt/acyclic/issues/41)) ([9a4cb73](https://github.com/iExalt/acyclic/commit/9a4cb73f8a083fa310db2fbde73f6bc435a2fcc1))
* make threads the unit of work and give the CLI a human side ([#6](https://github.com/iExalt/acyclic/issues/6)) ([cdbc6de](https://github.com/iExalt/acyclic/commit/cdbc6dee9b1b71189cb21bfd2a355ddc3769512f))
* **operator:** start runs with signed tokens and apply their writes once, as the run ([#13](https://github.com/iExalt/acyclic/issues/13)) ([f80a7a3](https://github.com/iExalt/acyclic/commit/f80a7a39fdb445d77f670173fb948f4b81bd8c2f))
* put threads with work left in waves of what to start first, and list each thread's waits ([#56](https://github.com/iExalt/acyclic/issues/56)) ([e6af979](https://github.com/iExalt/acyclic/commit/e6af979b4fe3c568acf1401c5603a0afd29388ad))
* read a parent as in progress while its open work's thread is claimed ([#43](https://github.com/iExalt/acyclic/issues/43)) ([963a80e](https://github.com/iExalt/acyclic/commit/963a80eebc66478ea5c947ba9caaf9aa42e9bd38))
* report threads two clones numbered alike, and renumber a fresh one ([#47](https://github.com/iExalt/acyclic/issues/47)) ([d33cc34](https://github.com/iExalt/acyclic/commit/d33cc34482098c1e46ed9eae881dd434f541441a))
* **secrets:** run Podman machines from mise and hand containers credentials as Podman secrets ([#11](https://github.com/iExalt/acyclic/issues/11)) ([85fe13c](https://github.com/iExalt/acyclic/commit/85fe13c2be66926f06dd07ca0d9dab2550b48c3c))
* **server:** let runs fetch and push their thread's code through the server, from mirrors of the forge ([#15](https://github.com/iExalt/acyclic/issues/15)) ([47c16f9](https://github.com/iExalt/acyclic/commit/47c16f9472d1701eca2b0162537dca75ff7328d9))
* **server:** serve runs' commands on a runners' listener, bound to their thread, scanned and applied once ([#14](https://github.com/iExalt/acyclic/issues/14)) ([e579069](https://github.com/iExalt/acyclic/commit/e579069ac076fb2ff9441a1736ba724d4ed3fcec))
* **server:** serve the tracker on the Mac with acyclic up, a session and an opt-in Tailscale socket ([#12](https://github.com/iExalt/acyclic/issues/12)) ([9b9690e](https://github.com/iExalt/acyclic/commit/9b9690e46fc870c0f6a311819be204dad235b914))
* tell each leaf's stage, and count a parent's leaves by stage ([#51](https://github.com/iExalt/acyclic/issues/51)) ([fbda0e5](https://github.com/iExalt/acyclic/commit/fbda0e5b61f8b3978422b656b7d8586cf449d413))


### Bug Fixes

* **app:** call the queue Needs you, as its notification does, so it doesn't read as the in-review stage ([#60](https://github.com/iExalt/acyclic/issues/60)) ([336e498](https://github.com/iExalt/acyclic/commit/336e498a2c8b80b4ced352d82926117bad8a8d24))
* **app:** close the menu alone, and badge thread numbers once ([#25](https://github.com/iExalt/acyclic/issues/25)) ([71f8938](https://github.com/iExalt/acyclic/commit/71f8938491cd4cf2426c45b6e3f2d9f5875501da))
* **app:** close the phases' sheet when Review opens, so its list isn't hidden under it ([#58](https://github.com/iExalt/acyclic/issues/58)) ([d50d015](https://github.com/iExalt/acyclic/commit/d50d015fdaff8af9ab627944552c92fbcfc98184))
* **app:** fix what using the app on a phone found ([#27](https://github.com/iExalt/acyclic/issues/27)) ([bc9decb](https://github.com/iExalt/acyclic/commit/bc9decb5404d67a73ff991e840f32f77ff61c50a))
* **app:** keep saved writes drawn past older reads, and drop stale autonomy drafts ([#29](https://github.com/iExalt/acyclic/issues/29)) ([f355711](https://github.com/iExalt/acyclic/commit/f3557112c91c6351ef939b5bc1b02738d3ff4065))
* **app:** let a phone's Back close what's open, one layer at a time ([#33](https://github.com/iExalt/acyclic/issues/33)) ([30de8bb](https://github.com/iExalt/acyclic/commit/30de8bb261f72dbb848972c12e531f05d5fb9ee6))
* **app:** let Escape close what's on top, keep typed input, and move focus with panels ([#32](https://github.com/iExalt/acyclic/issues/32)) ([718fbda](https://github.com/iExalt/acyclic/commit/718fbda2d92b08b4df7c4c09b22db6f4344b579e))
* **app:** let the next tap through after a menu action, and badge only named threads ([#26](https://github.com/iExalt/acyclic/issues/26)) ([3259e82](https://github.com/iExalt/acyclic/commit/3259e822ece5f127b13496e43855b8ca20bdc81f))
* **app:** pan from anywhere, open on the first item, and bring a picked thread into view ([#31](https://github.com/iExalt/acyclic/issues/31)) ([ad266ba](https://github.com/iExalt/acyclic/commit/ad266ba9dba04c7d6ab61920115eb4aae1a7e51f))
* **app:** redraw only what changed, and close the phone fixes' loose ends ([#28](https://github.com/iExalt/acyclic/issues/28)) ([e66a163](https://github.com/iExalt/acyclic/commit/e66a163cff5f82518e854e186cfc278a451ce690))
* **app:** save only what the user changed, and keep the plan when a refresh fails ([#30](https://github.com/iExalt/acyclic/issues/30)) ([c6a488f](https://github.com/iExalt/acyclic/commit/c6a488f6f93222288fab8eabb0720b3eddbd819c))
* **cli:** print readiness in pretty output as a phrase ([#9](https://github.com/iExalt/acyclic/issues/9)) ([22d514c](https://github.com/iExalt/acyclic/commit/22d514cde3bb485a58c2e139a2bf80265562a156))
* **core:** find thread cycles from the waves' own waits, so work in review holds a cycle and an unrecorded thread is in none ([#62](https://github.com/iExalt/acyclic/issues/62)) ([e0e2008](https://github.com/iExalt/acyclic/commit/e0e20084fdea1d62c29eb78861ad3d0bda07122c))
* **core:** find thread cycles only through open leaves, skipping archived work and a group's own thread ([#59](https://github.com/iExalt/acyclic/issues/59)) ([d3d3767](https://github.com/iExalt/acyclic/commit/d3d37678307f1c86410ea1a1231bdabda7fb36c6))
* point a missing page at the app, and drop CLI quoting from the app's words ([#34](https://github.com/iExalt/acyclic/issues/34)) ([161dde8](https://github.com/iExalt/acyclic/commit/161dde822ef34594bc329408971bc48152a780b9))
* **server:** share recent mirror fetches, bound a push's refs, and look repositories up within the run's thread ([#16](https://github.com/iExalt/acyclic/issues/16)) ([28e0d2a](https://github.com/iExalt/acyclic/commit/28e0d2a64106cfdd115d98c7196e7a942ed67064))
* **up:** keep each server's pairing apart on one host ([#37](https://github.com/iExalt/acyclic/issues/37)) ([8818208](https://github.com/iExalt/acyclic/commit/8818208442387fc4e6f4303c1dc0b0e6b5573139))


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
