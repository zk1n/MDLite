# Native regression harness boundaries

`Invoke-NativeTableAcceptance.ps1` と `Invoke-GuiAcceptance.ps1` は、製品の core-only fixture ではなく、起動した `MDLite.exe` の実 Win32/RichEdit 経路を観測する。ただし、合成メッセージ、OS 入力注入、native screenshot、Human 視認性は別の証拠として扱う。

## 追加した bounded checks

| Check | 実施 | PASS の意味 | BLOCKED / 未実施境界 |
|---|---|---|---|
| `continuous_unicode_input_no_accumulation` | Native table harness で 128 文字を `SendInput(KEYEVENTF_UNICODE)` 連続注入し、保存本文の長さ・内容を比較 | 対象 RichEdit に OS 入力が届き、重複・欠落なく一度だけ保存された | 前面 window を確実に取得できない、または `SendInput` が拒否された場合は `BLOCKED`。IME 変換単位は確認しない |
| `ctrl_z_single_transaction` | 表の行追加、列追加を別 transaction として実行し、OS-level Ctrl+Z を2回送信 | 1回目が直近の列追加だけ、2回目が行追加だけを戻す | Ctrl+Z の OS 注入が対象 window に届かない場合は `BLOCKED`。Human のキー感触・IME は未確認 |
| `arrow_repeat_boundary` | `SendInput` で右矢印を8回反復し、行境界を越えず本文を不変確認 | 反復入力が同一 table row の境界内に収まり、source mutation がない | foreground/input 注入失敗は `BLOCKED`。DPI ごとの caret 感触や長押しの OS repeat timing は未確認 |
| `table_paint_reentry` | table editor に 96 回の `InvalidateRect`/`UpdateWindow` を同期実行し、GDI/User handles を定点観測 | process が終了せず、client geometry と resource が bounded のまま | handle 閾値は leak の証明ではない。画像・表の主観的な見え方は Human gate |
| `paint_layout_reentry` | GUI harness で 64 回の同一サイズ `WM_SIZE` + main/editor repaint を実行し、client geometry と resource を観測 | native layout/paint 再入で geometry drift、process exit、明白な resource growth がない | OS DPI 切替、複数 monitor 移動、animation/resize の Human 感触は未実施 |

既存の table row/column、Tab、Shift+Tab、cell Undo/Redo、source 保存、compact、image、recovery の確認は維持する。`Shift+Tab` のように Windows `SendInput` 自体が `Win32 error 5` で拒否された場合は、製品 PASS へ昇格させず既存どおり `BLOCKED` とする。

## 実行例

```powershell
pwsh -NoProfile -File tools/Invoke-NativeTableAcceptance.ps1 -Preset release
pwsh -NoProfile -File tools/Invoke-GuiAcceptance.ps1 -Preset release
```

両スクリプトの JSON は `build/verification` 以下へ書く。これは native automated evidence であり、Microsoft IME/ATOK、100/125/150/200% DPI、複数 monitor、クリック/drag、table/image の視認性・操作感、PrintWindow screenshot の主観評価を PASS としない。