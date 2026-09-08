# Agentmux Mirror for Android

同一リポジトリ内のAndroidアプリです。PCのagentmuxが配信するagent一覧と画面を、Android 8以降で閲覧します。通常paneとACP transcriptの選択はPC側の `Tmux::pane_view` に共通化しています。ブラウザでも同じ画面を利用できます。

## PC側

```sh
cargo build --release
./target/release/agentmux serve --bind 0.0.0.0:9876
```

起動時に表示される接続トークンをAndroidに入力します。アプリの接続先には `http://PCのLAN IP:9876` を指定します。ブラウザの場合はそのURLを開き、トークンを入力してください。トークンは再起動ごとに変わります。固定する場合は32文字以上の英数字・`-`・`_`で構成したランダムな値をファイルに保存し、`--token-file /path/to/token` を指定します。

既定bindは `127.0.0.1:9876` です。LANへ公開するには上記の `--bind` が必要です。HTTPではトークンと画面が暗号化されないため、信頼できるLANで利用してください。ルーターのポート転送は不要です。必要に応じてPCのファイアウォールでLANからの9876/TCPを許可します。

## APKのビルド

JDK 17以上（検証用は21）、Android SDK API 36 / Build Tools 36.0.0を用意し、`ANDROID_HOME` または `local.properties` の `sdk.dir` にSDKのパスを設定します。

```sh
cd android
./gradlew assembleDebug
# app/build/outputs/apk/debug/app-debug.apk をAndroidにインストール
```

Gradle wrapperを同梱しています。AGP 9.2.1 / Gradle 9.6.1で固定しています。Androidのビルド要件は[公式AGPドキュメント](https://developer.android.com/build/releases/agp-9-2-0-release-notes)を参照してください。

## 動作と制約

- agentのみ列挙し、0件なら空状態を表示します。
- 約1秒ごとの画面スナップショット取得。切断時は表示をクリアして再試行します。
- ANSIの基本色・256色・RGB色、太字、斜体、下線と改行を表示します。端末の完全なエミュレーションや画像、カーソルの描画には対応しません。
- ACP transcriptを取得できない場合、通常のNeovim bufferへフォールバックしません。
- 閲覧専用です。Androidからの入力送信やPCのフォーカス変更は提供しません。
- 接続先のみ保存します。トークンは永続保存せず、アプリ再作成時に再入力します。
- WebViewは接続先のoriginに限定し、ファイルアクセスとJavaScriptネイティブbridgeを使いません。画面本文はHTMLとして解釈せず、テキストとして表示します。

## 配信インターフェース

`GET /api/agents` はagentに絞った `Snapshot`、`GET /api/view/%123` は `PaneView` のJSONを返します。両者とも `Authorization: Bearer <token>` が必須です。画面URLにはliteralなtmux pane IDのみ指定できます。通常paneの画面取得と更新系HTTP操作は拒否します。CORSは許可しません。

配信サーバーは4 worker、待ち行列16接続、ヘッダー8KiB、読み書きタイムアウト2秒に制限します。LAN内の少人数向けで、インターネット公開向けのサーバーではありません。

検証コマンド:

```sh
cargo test --all-targets
cargo build
python3 tests/remote_smoke.py
python3 tests/ui_smoke.py
node --check web/mirror.js
```
