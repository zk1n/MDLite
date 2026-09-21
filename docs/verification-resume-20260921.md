# 2026-09-21 MDLite UI再構築 継続検証

Task Inbox `20260921-mdlite-visual-rebuild-resume` Revision 1のimmutable snapshotを正本とした継続検証。
既存の実装・source authority・Undo/Redo・offline/local-first境界は変更せず、現HEADからfreshな証拠を取得した。

## Root / routing

- Resolver: `VERIFIED_ROOT`
- Project root: `E:\codex_work\MDLite`
- Branch / HEAD: `fix/visual-rebuild-localfirst` / `60bf614de4839026de59f0b2e60b0d370ba6d398`
- Effective Root: `gpt-5.6-luna / max`, policy `luna-max-gate`, selected profile `UNKNOWN`
- Jev live: `maintain`, confidence `0.64`, HTTP 200, `ROOT_CONTINUED`; specialistは起動していない。
- Task Inbox fetch: `TASK_INBOX_FETCHED`を1回、receipt `TASK.md` SHA-256 `7631a3027a786216523bc08970c3b013174f0ab4f5e5991307e838f58cd00a9a`。

## Fresh machine evidence

| Area | Result | Evidence |
|---|---|---|
| Debug build/core | PASS | `tools/Invoke-Build.ps1 -Preset debug -Test`; `All 301 MDLite core checks passed.` |
| Release build/core | PASS | `tools/Invoke-Build.ps1 -Preset release -Test`; `All 301 MDLite core checks passed.` |
| GUI acceptance | PASS | `build/verification/gui-acceptance-resume-20260921.json`, `pass=true`, Release SHA-256 `ad2e5bfbca40ff60d84594ab138ed76eee0fb552b5aecc7ede520ab317a5b767` |
| Quick performance | PASS | `build/verification/performance-release-resume-quick-20260921.json`; P1 six peak 28,581,888 bytes, P4 peak 42,266,624 bytes, compact 3/3, P5 100/100 failures 0, P5 input p95 max 25.626 ms |
| Native table | PASS with boundary | `build/verification/table-edit-native-resume-20260921.json`; row/column, cell Undo/Redo, Tab, append-row, arrows pass; Shift+Tab `SendInput 0/4`, Win32 5 before product branch, BLOCKED as harness/Windows input boundary. Core reverse-navigation remains PASS. |
| Native screenshot | PASS (evidence only) | `build/verification/native-screenshot-resume-20260921.json` / `.png`; `git_head` recorded, Release SHA-256 `ad2e5bfb...`, 1280x800, RichEdit 161 chars, `PrintWindow=true`, image SHA-256 `c6cb187d7ef99a3e6d86fa98a23094b5bf040b2429843471244f3a7aec353db1` |
| Local-first Git | PASS | `build/verification/git-local-first-resume-20260921.json`; isolated no-remote local commit, index-only boundary, untracked preservation |
| Diff / tracked secret scan | PASS | `git diff --check`; `git grep` scan found 0 secret/private absolute-path hits |
| Real holiday HTTP | PASS (fresh) | With `MDLITE_TEST_REAL_HOLIDAY_HTTP=1`, current Release core test ran twice; each returned `All 302 MDLite core checks passed.` exit 0. No credential/TLS/system-security change. |

## Visual and Human boundary

Read-only visual review found no mechanical clipping, negative bounds, or obvious pane/table layout defect in the available 1280x800 and responsive evidence. Native screenshot and geometry remain evidence-only. The remaining Human gate is limited to Microsoft IME/ATOK conversion and Undo feel, 100/125/150/200% DPI and multi-monitor movement, table/image visual quality and interaction feel, animation/resize, and outline/calendar click-drag. Computer Use could not run because the `orca` executable is unavailable in this environment; no Human PASS is inferred.

## External boundary

The historical `holiday-http-probe-20260921.txt` records WinHTTP 12185 from an earlier environment. The two fresh opt-in runs above supersede that result for this checkpoint; if the environment returns 12185 again, record BLOCKED without adding credentials or weakening TLS/security.

## Delivery boundary

User-owned untracked `samples/demo-workspace/.mdlite/` was not touched or staged. No routing files, other projects, main/tag/develop, force/rebase/reset/clean, signing, or binary publication were performed. Human gate remains before develop integration.
