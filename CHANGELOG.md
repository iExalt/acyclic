# Changelog

## [0.5.0](https://github.com/iExalt/acyclic/compare/v0.4.0...v0.5.0) (2026-11-02)


### ⚠ BREAKING CHANGES

* remove agent runs

### Code Refactoring

* remove agent runs ([9a86330](https://github.com/iExalt/acyclic/commit/9a86330db74376c3367c3e88ef12f3eeff653cd6))

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
