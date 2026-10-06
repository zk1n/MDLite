# Approved UI and remaining acceptance verification

Task: `20261004-mdlite-approved-ui-and-acceptance`, Revision 2.

Overall product acceptance is not PASS. This task preserves the prior implementation and closes a finite set of approved UI changes and remaining acceptance cases. Proposal mockups define the approved appearance changes; existing implementation defines available features. The proposal does not introduce new editor, template, outline, or Git commands.

## Implementation

- Standard Windows caption behavior remains enabled. Noninteractive client command-row space uses native caption hit testing; search remains interactive. Custom minimize/maximize/close buttons were removed in favor of OS controls.
- Twenty supplied original icon designs are retained under `assets/ui/icons`. `UiIcons.cpp` draws native geometry without an SVG/browser runtime. Activity names, tooltips, keyboard focus, and native selected state retain the existing open/focus commands; repeated clicks do not close panels.
- Git state text wraps in a readonly control and is measured before controls are placed vertically. Untrusted, no repository, missing Git, loading, failure, and normal status remain distinct.
- Calendar keeps month, selected day, document count, and the existing Daily action visible. Full paths and document lists remain available through details. A workspace without its optional metadata store shows unavailable count rather than inventing a count.
- Native tab/status controls retain layout and input APIs. Their client paint uses native rectangles and existing themed drawing to remove native raised frames. Editor presentation and Markdown source transactions are unchanged by these appearance changes.

## Build integrity

Localized MSVC include output had been misdecoded in generated Ninja rules, leaving zero recorded header dependencies and allowing stale objects after header changes. The existing build wrapper now discovers the prefix through the actual Ninja compile context, verifies an isolated header dependency, repairs only generated build-local rules, and requires recorded `Application.h` and `CalendarView.h` dependencies before writing a PASS receipt. A changed prefix causes one clean rebuild; an unchanged verified prefix does not.

The wrapper preserves failed-build receipt invalidation and source fingerprint checks before/after the build and records the resulting executable hash. Actual Japanese/accented path and isolated header-edit checks passed. No global locale, execution policy, authentication, or permission configuration was changed.

## Current checkpoint — 2026-10-05, source 5968

**Overall acceptance remains incomplete / NOT PASS. This verification checkpoint precedes Git/HANDOFF delivery; actual delivery is recorded separately in the final HANDOFF.** Current source is `5968e404fb1410c1c2be49ffafbc2c7bd363f53ea77a5448870958e9c538baee`. After the f01 integration checkpoint, the change is limited to `RenderGitPanel` display text: native readonly EDIT controls now receive CRLF line breaks. The broad f01 matrix remains evidence for f01; it is not relabeled as a run on 5968.

| Current 5968 evidence | Actual result | Identity / boundary |
|---|---|---|
| Debug build / CTest | PASS, 7/7 suites; 397 core checks | [Debug receipt](../build/verification/build-receipt-debug.json), EXE `b1b55e6b3a8ed6c26d0ef241bf0b601b1dc60f8c3575a896d4029e55928ffd88` |
| Release build / CTest | PASS, 7/7 suites; 397 core checks | [Release receipt](../build/verification/build-receipt-release.json), EXE `6fc5e932519d3987b53caf467b8ac0f68767c38dfebd3dbacf061034dc40b0b5` |
| Focused Git/E20 | [gui-final-crlf-e20.json](../build/verification/gui-final-crlf-e20.json): `pass=true` | Native-message/local temporary-index responsiveness route on the current Release EXE; not the full 19-case Git matrix |
| Save / profiles / settings | [Debug](../build/verification/save-final-crlf-debug.json), [Release](../build/verification/save-final-crlf-release.json): 9 PASS each, identity matches | Current source/executable stable before/after |
| Focused OS table navigation | Current Debug Shift+Tab 62→56 and forward Tab 56→62 PASS, collapsed selection | Exact source/Dirty/history/revisions unchanged; private Root key/caret receipts retained. This does not establish the remaining Unicode/Ctrl+Z/arrow-repeat cases |
| Current pixels | Root observed readable separate Git state/reason and accessible Trust in normal and 900×720 narrow views; Calendar six rows, Daily/details visible | Actual available display observations; no all-DPI/Human visual sign-off |
| Caption helper | Baseline PASS; two selected cases BLOCKED `CAPTION_COORDINATE_UNPROVEN`, all others NOT_RUN | UIA did not prove the maximize-button coordinate inside DWM bounds. This is not a current caption-gesture PASS or a foreground-failure diagnosis |
| IDE F5 | Current-source debugger path PASS | Actual OS F5 through existing VS Code preLaunchTask/cppvsdbg produced the requested demo-workspace process with `CheckRemoteDebuggerPresent=true` within the 60s budget. Fresh Debug receipt `08af5fa64db74bf6acfc1d06f1f94a3c`; source/EXE unchanged. Stop and owned Code-window cleanup confirmed; 15 selected stable sample files unchanged, 0 added. F5 pixel capture remains BLOCKED by the provider's window-not-found result; the first helper's no-input foreground blocker remains retained |

No full GUI, full P0–P5 or ATOK matrix was repeated for the display-string-only delta. The f01 failures, blockers and measurement limits below remain open; focused current PASS does not establish whole-task acceptance or delivery.

## Preserved integration checkpoint — source f01

This integration ran on source fingerprint `f01aedf35ba08237f297b063358309fb786da75f0dfc3a65874a298e69cbd5cb` (50 files). Build, native-message routes, OS input, real IME, visual acceptance, and performance retain separate evidence boundaries.

| Configuration | f01 build / CTest | Executable SHA256 |
|---|---|---|
| Debug | PASS, 7/7 suites; 397 core checks | `abe2aa6ca770ccf494903f4240a1b67ccc666934ea45695ef15d6f54f3ab08a9` |
| Release | PASS, 7/7 suites; 397 core checks | `fce007a9bda21188a6d2565eaee2d82689d9bf50805ee54d506ecccc3f68092e` |

The f01 receipts are preserved separately; mutable current receipt filenames now refer to 5968. f01 identities are also retained in the Save artifacts and the Calendar/Git embedded Release receipt. Both f01 builds passed source/executable identity and recorded-header dependency gates.

| f01 lane | Actual artifact result | Boundary / remaining case |
|---|---|---|
| Save / profiles / settings | [Debug](../build/verification/save-final-f01-debug.json) and [Release](../build/verification/save-final-f01-release.json): 9 PASS each, `identity_matches=true` | Exact empty filename/zero bytes, profile persistence, Cancel/Apply/restart and the existing regression cases; native messages and disk/model observations, not physical input |
| Calendar Daily / creation / Rename / Move | [calendar-daily-final-f01.json](../build/verification/calendar-daily-final-f01.json): 9 PASS, source/executable unchanged | Native-message product routes and bytes/index/session observations; no Human/pixel/DPI or physical-input PASS |
| Git | [git-final-f01.json](../build/verification/git-final-f01.json): 18 PASS / 1 BLOCKED, raw `pass=false` | Trust, isolated repository states, Stage/Unstage, Commit and loopback Fetch cancellation pass. `command_palette_custom_binding_escape_editor` remains BLOCKED by the 10s native palette-readiness timeout. Fixture commits are not project-repository delivery |
| Native table | [native-table-final-f01.json](../build/verification/native-table-final-f01.json): 15 PASS / 4 BLOCKED | Shift+Tab, continuous Unicode, Ctrl+Z and arrow repeat are `BLOCKED_FOREGROUND_NOT_ESTABLISHED`; current OS-input success is not established |
| Full GUI | [gui-final-f01-release.json](../build/verification/gui-final-f01-release.json): raw `pass=true`, all 237 top-level Boolean fields true | Release executable matches f01 receipt; includes session viewport and Calendar restoration. This is the finite synthetic native-message regression, not overall physical-input/real-IME/Human acceptance |
| Real ATOK | Four finite f01 cases PASS with Root-observed actual preedit/commit/reconversion pixels and source/history readback | Commit/Undo/Redo, composition cancellation on a clean baseline and reconversion pass. The first raw cancellation result remains `FAILED_STEP` because autosave advanced saved revision 1→4 while source/history stayed unchanged; the controlled clean-baseline case preserves all fields without settings changes. Private artifacts remain local; Microsoft IME and physical input are separate |
| Full Release P0–P5 | [performance-final-f01-full-100.json](../build/verification/performance-final-f01-full-100.json): `FAILED_OR_INCOMPLETE`, source stable; two P2 startup-query failures | P5 completed 100 iterations with 0 failures. P2 input/search/cancellation measurements were not reached; no overall performance PASS |

Performance values below are the actual scenario fields. Working-set and private-byte peaks are maxima of recorded samples; early main-window appearance is separate from document readiness. Input p95 measures synchronous `WM_CHAR` return and excludes deferred source synchronization/presentation. P0 first-run approximation/warm main-window times were 89.426 / 55.732 ms with no document loaded; P4 completed its measured image/compact stages with a sampled working-set peak of 44.86 MiB.

| Scenario | Document ready | Sampled working-set peak | Sampled private-byte peak | Observed input p95 |
|---|---:|---:|---:|---:|
| P1 one document, 1 MiB open source | 38,921.802 ms | 42.27 MiB | 22.05 MiB | 49.780 ms |
| P1 six documents, 1 MiB aggregate open source | 33,207.226 ms | 39.79 MiB | 18.83 MiB | 52.078 ms |
| P3 20 MiB source | 6,485.145 ms | 254.20 MiB | 255.86 MiB | 368.221 ms |
| P3 100 MiB source | 16,295.086 ms | 1,103.09 MiB | 1,195.66 MiB | 2,358.373 ms |

P1's recorded one/six-document memory samples are below the 50MB typical-workload target; this does not establish that target for every workload or startup peak. P3 confirms only the measured large plain-source load/edit/save workload, including 51 synthetic ASCII characters and exact suffix round trips. It does not establish large-document rich-feature or real-IME performance. P2 has 10,000 generated files / 200 MiB, with 1 MiB open source in either the one- or six-document case. Both fail at the startup tab-count query with a configured 60,000ms `SendMessageTimeout` budget, `Win32=0`, `hung=true`; failure snapshots record CPU 14,796.875 / 38,703.125 ms and working sets 31,621,120 / 33,378,304 bytes. Those snapshots are not completed-load memory peaks. The exact UI-thread bottleneck remains unproven; tree/shell/layout and document rendering require a bounded diagnostic follow-up, without treating the failure as a successful search measurement. P5 first/last private bytes are 5,459,968 / 5,480,448 (+20 KiB); 100 successful cycles do not prove absence of leaks in the failed P2 workload.

All GUI, EditHistory, ATOK and IDE F5 observations from f5 or earlier remain historical. The f5 GUI artifact retains failed session-readiness and Calendar-restoration subcases; a later focused f5 Calendar pair passes, while session readiness still fails within its original budget and viewport assertions are not reached. Historical f5 EditHistory is 44 PASS / 5 FAIL / 5 BLOCKED. None is relabeled as f01 or 5968 acceptance. f01 ATOK evidence above belongs to f01. Microsoft IME, physical-input coverage, unresolved editor/Git/performance cases, Human/DPI conditions, and delivery remain open.

## Historical checkpoint — source f5

Current source fingerprint: `f5aa7bd422174c41193c378c9d70921ff0a13ef8b77a1716a933c2aabee5d585`, 50 files.

| Configuration | Build / CTest | Executable SHA256 |
|---|---|---|
| Debug | PASS, 7/7 suites; core 397 | `ac9576eea2107629aeab667c2a6b0c0e4370a8ea1785270d30837d3c3c29ea51` |
| Release | PASS, 7/7 suites; core 397 | `42a9f4401cb1ea19ac64dcdb0fddb8bbacf06236d44d7ed0699d1f9adac5b120` |

An actual OS Shift+Tab run on the preceding source moved from body cell two to a selected header, instead of the previous body cell. Source inspection shows character translation after the custom keydown handler consumed Tab; the observed header selection matches an additional native navigation step. The exact WM_CHAR trace was not captured. The new 15-line change dispatches editor Tab keydown first and skips character translation only when that existing custom handler consumes the key. Its signal resets before and after dispatch. Ordinary Tab, IME, Unicode packets and other key paths retain their routes. A separate reviewer found no concrete correctness/source-safety issue in this patch.

Build and noninteractive CTest passed on the new source. Fresh actual OS Shift+Tab moved source62 to56 collapsed, and Ctrl+Z restored one transaction. Save/profile/settings passed all nine cases in both Debug and Release with matching source/executable identities. Calendar passed all nine Daily/index/creation/Rename/Move cases. The first full Release GUI rerun has 235 true checks and two unresolved subcases (session readiness and Calendar menu restoration). Unicode post-input settlement, ordinary Tab/IME and remaining final GUI integration are still under verification. The legacy same-row arrow criterion remains failed and is tracked as ZK1-12; its semantic conflict is deferred, not converted to PASS. The records below belong to the earlier executable and remain historical. F5, final Release performance, Git delivery and HANDOFF remain pending. No whole-task acceptance or Inbox cleanup is claimed.

## Historical UI baseline evidence

This section preserves earlier observations. Each result belongs to the source recorded by its artifact; subsequent pre-f01 observations do not inherit the baseline fingerprint merely because they appear below this heading.

Source fingerprint: `0bc003a6cbb8b45750f03f99756848e582d35c7e23ab9d73d405ac878830f0fd`, 50 files.

| Configuration | Build / CTest | Executable SHA256 | Boundary |
|---|---|---|---|
| Debug | PASS, 7/7 suites | `832e74ef39879c8edcf9a48ecd178d7ad3f616a1da71ae43a7da75aa2e91d9af` | Source/executable receipt and actual header dependencies verified |
| Release | PASS, 7/7 suites | `bfe8f0eafaf4a21ff5d3ce314b33834bc0ebd452098ee5c39103c986986a0429` | Same source; incremental dependency tracking verified |

Core suite: 397 checks. Build/CTest PASS does not establish GUI, IME, clipboard, performance, or whole-task acceptance.

Save/profile/settings native regression: Debug and Release each passed all nine existing cases with matching source/executable identity. Empty Save As preserves the requested filename, zero bytes, and clean revision; Profile Apply verifies its exact confirmation, complete persisted definition, and captured-dialog acknowledgement. The observed native single-OK message box uses button ID2; acknowledgement follows the actual semantic OK control rather than assuming ID1. Final isolated startup observations were 5,861–7,447 ms (Debug) and 5,984–8,203 ms (Release), including metadata initialization. These are harness observations, not the full performance profile.

Release Git fixture passed 18 native cases, including Commit's exact HEAD, parent, subject, staged content and unchanged dirty editor. The original Fetch-cancellation case passed through a separate normal process: actual progress-dialog Cancel, exit 1223, unchanged HEAD/index/status/worktree/editor, and confirmed removal of the fixture's own real Trust grant. Git creates an empty FETCH_HEAD before contacting the remote; the original raw result retains that difference and its false overall flag. A separate revalidation permits only this absent-to-empty administrative file change. Full before/after compressed-object bytes were not captured and are not claimed. Together these records cover the original 19 finite Git cases; fixture commits are not repository delivery.

Release Calendar passed all nine existing native cases, including Rename and Move with unchanged document bytes, creation provenance and index. The native file dialog exposes its filename through FileNameControlHost and a writable Edit ID1001; its Save control is bound from the actual dialog. An initial failed active-document assertion ran after intentionally closing Daily. The retained raw result records that failure; the corrected check runs before the close and the complete rerun passes.

Native OS-injected caption movement, double-click maximize/restore, maximize-and-drag restore, edge Snap, eight resize directions and Alt+Space menu passed with unchanged source/history/selection. Caption minimize/taskbar restore and Win+Z flyout remain unproved. Final pixels confirm the flat tab/status/editor treatment, retained native caption, unified icons, readable Git layout and initialized Calendar count. These observations do not establish physical input or all display configurations.

Calendar hover passed a 30.27-second synchronous native-message run on the final Release executable. Panel layout passed all ten native-message cases after waiting for the completed workspace caption before actions and restart readback. The earlier raw restart failures sampled created but uninitialized controls and remain retained.

Actual ATOK input passed four finite cases: visible hiragana preedit with unchanged model/history, exact Enter commit in one transaction, cancellation preserving model/history, and real selected-text reconversion candidates with cancellation. Ordinary OS Ctrl+Z/Ctrl+Y restored the pre-commit and committed text in one step. Root inspected the actual preedit, prediction and reconversion pixels; the configured Japanese TIP is ATOK36 and the owned thread uses the Japanese input layout. These tests did not inject IME messages or Unicode packets and did not change input-method configuration. Microsoft IME remains disabled in the existing configuration and was not activated. Physical input and long-press remain untested.

Final broad GUI and edit regressions currently retain failed and blocked raw results. Identified driver issues include source/display-offset confusion, early dialog/workspace readback, and stale native tab/full-selection expectations. Remaining clipboard/caret observations are under bounded diagnosis. Correctly constructed OS keyboard events now prove the table Ctrl+Z and arrow-repeat cases. Shift+Tab selects an unexpected header range, and Unicode input moves the caret without producing the expected source; both are retained failures. Partial keyboard insertion cleanup records queued key-up counts, not observed key-state recovery. These partial results do not establish overall acceptance.

## Finite acceptance scope

| Cases | Required product path | Evidence boundary |
|---|---|---|
| SAVE01–03 | Requested empty Save As, profile confirmation/persistence, existing nine save/settings cases | Native dialog/control and byte/state readback; strict filename and persistence checks |
| UI01 | Move, maximize/restore, drag restore, resize, system menu, caption buttons and Snap | OS-injected coordinate/key input with window-state readback; physical input remains separate |
| UI02–04 | Icons/names/tooltips, interaction states, thin borders/margins/status | Native screenshots, accessibility/control state, keyboard and tab regression |
| UI05–06 | Git states/trust/actions and Calendar activation/details/rename/move | Isolated fixtures; no operations on the user's real repository |
| UI07 | Normal/compact/narrow, themes and available DPI | Actual display conditions separated from synthetic DPI messages |
| EDIT01–02 | History, table, Unicode/CRLF, directional selection, caret, clipboard and OS input | Source/Dirty/history/selection/bytes and owned clipboard cleanup |
| IME01–02 | Microsoft IME and ATOK composition/commit/cancel/reconversion | Real installed IME observations; Unicode injection is not IME evidence |
| GUI01 / DEV01 | Existing GUI/recovery/settings regression and F5 debugger path | Actual native app and IDE launch; CLI build is not F5 PASS |
| BUILD01 / PERF01 | Both build configurations and final Release P0–P5 | Current receipts, full fixtures and actual measurements |
| REVIEW01 / DELIVER01 | Ponytail, one separate correctness/safety reviewer, normal Git and HANDOFF delivery | Reviewer separate from implementation; dual-ref and upload/readback proof |

Each final artifact must identify source/executable, fixture, input method, expected/observed result, and PASS/FAIL/BLOCKED/NOT_RUN. Prior source results remain historical. Physical long-press, subjective visual checks, unexecuted DPI/monitor conditions, and real IME evidence are not promoted by synthetic tests. Additional unrelated defects become follow-up issues rather than extending this case set.

This feature-only verification checkpoint records the completed evidence above, before Git/HANDOFF delivery. Whole-task acceptance is NOT PASS and the develop-integration condition is unmet. Actual delivery results belong to the final HANDOFF. Historical failures, blocked routes and the original finite criteria remain preserved.

## Resumed prototype checkpoint — 2026-10-06, source 0d9f

All pending/in-progress labels in this historical section describe the checkpoint time. This section adds a source-qualified prototype cycle; it does not rewrite the historical 5968, f01, or f5 results above. The build artifacts identify source fingerprint 0d9f4cb4444b5bd18d80034f1338ae663809e2c615f70fcd27feceafd98f2aa0 (50 source inputs). Debug and Release build/CTest both PASS, 7/7 suites. Debug build run 0d851e80827f4cf18243f724f439bb57 produced EXE SHA-256 7a322ac88fe861503c26a78f1f6cb2eae9388749c58d21ed7be1230123803abc; Release build run 20fc3337df33480abdb7cd3b072fa193 produced EXE SHA-256 db8d3fc60ce3c03a2efb66c86a1a77301250d06cfd92eefd094b160791b8e8df.

| 0d9f lane | Current result | Boundary / next step |
|---|---|---|
| Debug and Release build/CTest | PASS, 7/7 each | Receipts and EXE hashes above bind both configurations to source 0d9f. Any source change requires fresh receipts. |
| P2 one-document startup-only | FAIL / STARTUP_TIMEOUT at the 60,000 ms bound | Root's latest trace records Calendar index 43,969 ms, Tree 24,050 ms, total Workspace 73,498 ms. These are trace phase spans, not totals to add. High is investigating the causal slice; this is not a performance PASS. |
| Controlled clipboard comparison | Partial: fixture text exact; cross-process WM_PASTE left source unchanged after 5 s; OS Ctrl+V injected 4/4, then BLOCKED with SOURCE_PRESENTATION_NOT_READY at the helper's 250 ms presentation observation | The OS result is unknown, not PASS or FAIL. Root confirmed restoration of the captured actual clipboard baseline. The earlier pre-730 original value remains UNKNOWN and is not retroactively established by the 373 capture. Publisher/readback review remains pending; do not repeat the broad clipboard/cut harness. |
| Human preview | PASS for a scoped visual preview only | PreviewInfo binds source 0d9f, Release build run 20fc3337df33480abdb7cd3b072fa193, EXE SHA-256 db8d3fc60ce3c03a2efb66c86a1a77301250d06cfd92eefd094b160791b8e8df, and commit ab3762702056f8fb8d2e58e1bf343aa49b31f50f with a dirty worktree. Root observed readable pixels, native caption, Activity, table/Git text, Calendar's five-file count, and Calendar Details/creation information. The owned process exited after normal WM_CLOSE. No editor, Trust, or IME settings were changed. |
| Full GUI, Save, Native Table, Edit History | NOT RUN on source 0d9f after these receipts | Run the finite suites once after High's fix is integrated and both receipts match the final source. Keep clipboard cases filtered and gated on the publisher review. |
| Full Release P0–P5 | NOT RUN for the final source | Reconsider one full 100-iteration run only after the focused P2 result and regressions justify it. |

The preview used the same fix branch at commit ab376 with source 0d9f in a dirty worktree. It is an engineering prototype observation, separate from user configuration changes and whole-product acceptance. The final post-commit source/build and any source-changed human preview remain pending. Overall acceptance remains NOT PASS.

## Historical engineering candidate — source b768

All current-source and pending/in-progress labels in this historical section describe the b768 checkpoint time. Source `b76827b8087c3b1d2390a6865faa98333d6a3ccb4e33757551e362ec54a3b4b7` includes raw table-cell padding preservation, batched Explorer redraw, and cancellable Calendar indexing. The Calendar result is applied on the UI thread only for the current workspace/generation; explicit date actions refresh completed indexes. Iterator and delivery errors keep partial results out of the ready state. The separate reviewer found no remaining concrete code blocker after three Calendar corrections.

| Current-source verification | Result | Scope |
|---|---|---|
| Debug / Release build and CTest | PASS, 7/7 each | Debug EXE `988ec553b53e64731f0c083e57426c9ade65559eb1e35ea8f7c5675ed1c4f202`; Release EXE `e27c2e866de847e3368c32ff3a0912ad970ed84d85d6554fabe74181994e9f36` |
| Save / Profile / Settings | PASS, 9/9 each configuration | Source and executable identity match before and after the native runs |
| Real clipboard, WM_PASTE and OS Ctrl+V | Exact effect PASS in separate fake fixtures | Source, Dirty, one undo transaction, collapsed caret and clipboard generation verified; OS batch accepted 4/4. Captured current baselines restored. WM fixture cleanup required a separately recorded normal-close follow-up; OS fixture completed normal cleanup. The earlier test-helper 730 original clipboard state remains UNKNOWN |
| P2 startup and Calendar publication | PASS within the original 60 seconds | Editor ready at 24,454.763 ms; accepted full 10,000-file Calendar index observed at 52,920.208 ms. Normal owned-process exit and build identity verified; full P0–P5 remains in progress |
| Broad GUI | 233/236 Boolean conditions true; overall incomplete | Two underlying paths remain in the retained raw result: legacy viewport observation and Save retry observation. Corrected bounded observers await focused runtime confirmation |
| Native Table | Automated product checks PASS; four OS cases BLOCKED | Shift+Tab, continuous Unicode input, Ctrl+Z and arrow repeat lack foreground execution evidence in this run |
| Filtered nonclipboard Edit History | 31 PASS, 0 FAIL, 3 BLOCKED | Outline drag/notification delivery blocked with Win32 1460/5. Initialization still reads the clipboard; no selected case published or restored it |

Final ATOK, F5, caption/pixel checks, focused GUI reconciliation, final version and Git/HANDOFF delivery remain pending. Physical input, disabled Microsoft IME, other display configurations and Human review remain distinct. Whole-product acceptance is NOT PASS.

## Current engineering candidate — source c269

Source fingerprint: `c2694697ee91768644a40562ff4b5018bd2c8a44a20405f5342e869ea6b27433`, 50 inputs. In addition to the b768 changes, clean saved tabs hydrate without repeatedly activating the editor. Layout and settings resolve first; source/history and inactive viewport anchors remain preserved. The saved active document activates once. Dirty recovered documents retain the resolved autosave scheduling. The separate correctness/safety reviewer found no remaining concrete code blocker in this delta; 21 extracted-method checks passed.

| Current-source verification | Result | Boundary |
|---|---|---|
| Debug / Release build and CTest | PASS, 7/7 each; core 397 | Debug EXE `27365df1ed38b676afcf519c76b607e1182cf23f4021ba8cff000eb6e234af63`; Release EXE `0800da84c4c93cb72d5a572bb34dc55c1d778d6e510b3fccbda3f52759315794` |
| Broad GUI | PASS, all 236 Boolean conditions | Current Release executable; native harness results do not establish physical input or every display configuration |
| Save / Profile / Settings | PASS, 9/9 in each configuration | Matching source/executable identities before and after both native runs |
| Focused session viewport and Save retry | PASS, both cases | Original 10/5-second observer bounds retained. The separate focused fixture needed guarded forced cleanup of its own synthetic process, which is not normal-close acceptance |
| Release full P0–P5 | PASS for the measured profile | Current-source full run: zero scenario failures; P5 100 iterations, zero failures, private bytes +69,632 (68 KiB). Search settlement is a 750ms no-count-change heuristic, not completion-generation proof |
| Real ATOK | PASS, four finite cases | Current Debug: inspected actual preedit/reconversion pixels; exact commit/cancel/source/history and OS Undo/Redo. No IME configuration changes; owned process normal-close verified |
| Native caption buttons | PASS, maximize and restore | Actual screenshot-directed OS button clicks and window-state readback; source/Dirty/history/revision/selection unchanged. Other gestures and physical/DPI conditions remain separate |
| Actual OS table input | Shift+Tab and Ctrl+Z PASS; Unicode and arrow finite conditions remain FAIL | Current-source owned input and source/history readback. Shift+Tab follows a separately retained initial foreground BLOCKED run. Arrow59→70 preserves source but fails the original row criterion (ZK1-12). Unicode224/224 is accepted, but the original shared10s exact197/save condition remains incomplete; Jev third-round defer preserves the failure |
| Current clipboard comparison | BLOCKED | WM setup/focus prevented publication; OS-only exact fake readback refused at OpenClipboard before paste. The OS probe restored its eagerly captured current baseline and normal-closed its owned fixture. No current clipboard effect PASS; preceding b768 WM/OS results remain historical |
| Current F5 | BLOCKED before F5 input | Owned IDE appeared, but Root could not attest the selected configuration within the separate60s setup interval. No F5 sent, no fresh debugger proof. Owned Code window normal-closed; launch/tasks/security unchanged |
| Other broad Native Table / filtered Edit observations | Historical b768 coverage only | Unchanged routes were not rerun merely to change the source label; current CTest, broad GUI/table topology and focused OS/ATOK/save evidence cover the changed claims. Outline notification routes remain blocked in the earlier artifact |

Whole-task acceptance remains NOT PASS. Human physical input, subjective review, disabled Microsoft IME, and untested display configurations remain separate. The original test-helper 730 clipboard state remains UNKNOWN; successful later snapshot restoration does not recover that earlier state. Git/develop/HANDOFF status will be recorded from actual delivery results.

Current measured P1 one/six-document working-set peaks are 48,447,488 / 44,421,120 bytes; document readiness is 9,272.714 / 13,090.265 ms. P2 document readiness is 29,384.493 / 38,853.935 ms; search-settled heuristic is 295,927.944 / 278,914.064 ms. P3 20/100MiB input p95 is 352.426 / 2,056.898 ms, with working-set peaks 270,876,672 / 1,158,942,720 bytes. Profile PASS does not imply low latency for large documents or all performance goals.

Unicode diagnostic detail: baseline exact read took110.713ms/99RPCs; the input-queue phase took4,267.683ms. Six-scalar polling completed198RPCs, with0full197 scans because a settled candidate was not reached before10s. The latest partial source length181 is not a final-length or permanent-loss result. The preceding observer produced torn117/137 snapshots with at least530RPCs. The original FAILs remain retained; no deadline, source, caret, disk or history condition was relaxed. Further diagnosis is deferred within the existing editing issue, and overall acceptance stays NOT PASS.
