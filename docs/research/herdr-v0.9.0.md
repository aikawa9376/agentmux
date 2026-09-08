# Herdr追従・性能調査（2026-09-08）

## 比較元と範囲

本家は [herdrdev/herdr](https://github.com/herdrdev/herdr) に移転。
[CHANGELOG](https://github.com/herdrdev/herdr/blob/9e01168b140ce8e3821131345dc82bc2bf9994eb/CHANGELOG.md) の最新releaseはv0.9.0（2026-09-07）。今回取得したmasterは `9e01168b140ce8e3821131345dc82bc2bf9994eb`。

v0.7.3以降の更新には複数マシン集約、client独立表示、検出改善などがある。agentmuxではtmuxがPTYとlayoutを所有するため、今回は既存sidebarへ直接適用できる検出更新と監視負荷を対象とした。複数マシン管理、worktree、通知、daemon、unseen doneなどは未実装のまま。全機能parity完了を意味しない。

## 実装

- 本家のmanifest差分を取り込み、Maki・Muse・Qwenを追加。計21 manifest。ClaudeのMCP質問・background agent、Copilotのbackground待ち、Codexのtrust/update dialogと引用された確認文の誤判定などをfixtureで検証。
- Copilotのagentmux固有folder-trustルールを維持。Codexの古い `after_last_prompt_marker` 拡張は本家の `whole_recent_without_current_prompt_marker` に置換。current prompt中の `[y/n]` はidle、promptの後に実行block markerがある生きた確認はblockedとなる。
- engine v3の `top_non_empty_lines` を追加。CRLFをLFと同じ1byteとして数える旧offset計算を修正し、日本語を含むsliceの破損を防止。
- `canonical_kind` が毎回全候補のTOMLをparseしていた処理を、既存の `OnceLock` compiled manifestへ統合。nested gate間で小文字化結果を共有。
- 非表示previewのcaptureを停止。control-mode通知queueは256件、1 frameのdrainも256件に制限。満杯なら通知を捨ててreaderを止めず、既存通知によるsnapshot更新で再取得する。
- 無出力でも `max(status_ms, 2000)` msごとにsnapshotを再取得。pane option変更、TTL失効、owner終了、watcher切断を回復する。busy時は従来のstatus_msで制限。

## 性能測定と検証

`examples/detection_bench.rs` は4種の入力をwarm-up後に反復する再現用benchmark。
同一環境のdebug build、`cargo run --example detection_bench -- 500`（2,000判定）では変更前16.549秒、変更後0.02728秒。manifestの内容も更新しているため純粋なcache単体比較ではなく、起動時compile・tmux subprocess・描画を含まない。UI全体の速度倍率には換算できない。release測定は `cargo run --release --example detection_bench -- 10000`。

35 unit testsと8 integration tests、`cargo clippy --all-targets -- -D warnings`で検証。tmux integrationは専用socketで実行し、sandboxによるskipなしを確認。

## 継続する制約

- OSC progressの入力は未取得。該当ruleは空文字を受けて一致せず、screen/title fallbackを使う。tmux pane_titleが上書きされる環境ではタイトル判定の信頼性は限定的。
- `skip_state_update` はstateless snapshotではunknownを返す。本家の前回state維持やunseen doneとは異なる。
- snapshotは同期処理で、全process走査とagent paneごとのcaptureが残る。今回の判定cacheで改善した後も、大量pane環境では次の計測候補。background worker化はstate所有権と更新競合を設計してから行う。
- upstream最新版のlicenseはApache-2.0。今回の更新分のlicenseを同梱し、旧AGPL由来コードの表記とプロジェクトlicenseを維持（NOTICE参照）。
