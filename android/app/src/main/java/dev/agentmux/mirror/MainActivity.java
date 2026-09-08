package dev.agentmux.mirror;

import android.app.Activity;
import android.os.Bundle;
import android.net.Uri;
import android.text.InputType;
import android.view.inputmethod.InputMethodManager;
import android.content.Context;
import android.webkit.WebResourceRequest;
import android.webkit.WebResourceError;
import android.webkit.WebResourceResponse;
import android.webkit.WebView;
import android.webkit.WebViewClient;
import android.widget.*;
import java.io.ByteArrayInputStream;
import java.net.URI;

/** Native connection screen with a same-origin, read-only mirror WebView. */
public final class MainActivity extends Activity {
    private WebView web;
    private LinearLayout root;
    private EditText address;
    private EditText token;
    private TextView status;
    private String origin;

    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        showConnection();
    }

    private void showConnection() {
        if (web != null) { web.stopLoading(); web.destroy(); web = null; }
        origin = null;
        root = new LinearLayout(this);
        root.setOrientation(LinearLayout.VERTICAL);
        int pad = (int)(16 * getResources().getDisplayMetrics().density);
        root.setPadding(pad, pad, pad, pad);
        root.setOnApplyWindowInsetsListener((v, insets) -> {
            v.setPadding(pad + insets.getSystemWindowInsetLeft(), pad + insets.getSystemWindowInsetTop(),
                pad + insets.getSystemWindowInsetRight(), pad + insets.getSystemWindowInsetBottom());
            return insets;
        });
        TextView title = new TextView(this); title.setText(R.string.text_1); title.setTextSize(24); root.addView(title);
        address = new EditText(this); address.setSingleLine(true); address.setHint(R.string.text_2);
        address.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_URI);
        address.setText(getPreferences(MODE_PRIVATE).getString("address", "")); root.addView(address);
        token = new EditText(this); token.setSingleLine(true); token.setHint(R.string.text_3);
        token.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_PASSWORD);
        token.setSaveEnabled(false); root.addView(token);
        Button connect = new Button(this); connect.setText(R.string.text_4); root.addView(connect);
        status = new TextView(this); status.setText(R.string.text_5); root.addView(status);
        connect.setOnClickListener(v -> connect()); setContentView(root);
    }

    @SuppressWarnings("SetJavaScriptEnabled")
    private void connect() {
        String value = address.getText().toString().trim();
        String secret = token.getText().toString().trim();
        try {
            URI uri = new URI(value);
            if (!("http".equals(uri.getScheme()) || "https".equals(uri.getScheme())) || uri.getHost() == null
                || uri.getUserInfo() != null || uri.getQuery() != null || uri.getFragment() != null
                || !(uri.getPath().isEmpty() || uri.getPath().equals("/")) || uri.getPort() > 65535) {
                throw new IllegalArgumentException();
            }
            if (!secret.matches("[A-Za-z0-9_-]{32,}")) { status.setText(R.string.text_6); return; }
            origin = uri.getScheme() + "://" + uri.getRawAuthority();
        } catch (Exception e) { status.setText(R.string.text_7); return; }
        getPreferences(MODE_PRIVATE).edit().putString("address", origin).apply();
        ((InputMethodManager)getSystemService(Context.INPUT_METHOD_SERVICE)).hideSoftInputFromWindow(token.getWindowToken(), 0);
        token.setText(R.string.text_8); root.removeAllViews();
        Button back = new Button(this); back.setText(R.string.text_9); back.setOnClickListener(v -> showConnection()); root.addView(back);
        status = new TextView(this); root.addView(status);
        web = new WebView(this); web.setBackgroundColor(0xff1e1e2e);
        web.getSettings().setJavaScriptEnabled(true);
        web.getSettings().setAllowFileAccess(false); web.getSettings().setAllowContentAccess(false);
        web.getSettings().setCacheMode(android.webkit.WebSettings.LOAD_NO_CACHE);
        web.setWebViewClient(new WebViewClient() {
            private boolean allowed(Uri uri) {
                try { URI target = new URI(uri.toString()); return origin != null && origin.equals(target.getScheme() + "://" + target.getRawAuthority()); }
                catch (Exception e) { return false; }
            }
            @Override public boolean shouldOverrideUrlLoading(WebView view, WebResourceRequest request) { return !allowed(request.getUrl()); }
            @Override public WebResourceResponse shouldInterceptRequest(WebView view, WebResourceRequest request) {
                if (allowed(request.getUrl())) return null;
                return new WebResourceResponse("text/plain", "UTF-8", new ByteArrayInputStream(new byte[0]));
            }
            @Override public void onReceivedError(WebView view, WebResourceRequest request, WebResourceError error) {
                if (request.isForMainFrame()) status.setText(R.string.text_10);
            }
        });
        root.addView(web, new LinearLayout.LayoutParams(-1, 0, 1));
        web.loadUrl(origin + "/#token=" + secret);
    }
    @Override protected void onPause() { if(web != null) web.onPause(); super.onPause(); }
    @Override protected void onResume() { super.onResume(); if(web != null) web.onResume(); }
    @Override protected void onDestroy() { if(web != null) { web.destroy(); web = null; } super.onDestroy(); }
}
