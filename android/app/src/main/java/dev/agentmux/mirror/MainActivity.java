package dev.agentmux.mirror;

import androidx.activity.ComponentActivity;
import androidx.activity.result.ActivityResultLauncher;
import androidx.activity.result.contract.ActivityResultContracts;
import com.journeyapps.barcodescanner.ScanContract;
import com.journeyapps.barcodescanner.ScanOptions;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
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
public final class MainActivity extends ComponentActivity {
    private WebView web;
    private LinearLayout root;
    private EditText address;
    private EditText token;
    private TextView status;
    private String origin;
    private boolean readingImage;
    private final ExecutorService imageWorker = Executors.newSingleThreadExecutor();
    private final ActivityResultLauncher<ScanOptions> camera = registerForActivityResult(
        new ScanContract(), result -> {
            if (result.getContents() != null) acceptPairing(result.getContents());
        });
    private final ActivityResultLauncher<String> gallery = registerForActivityResult(
        new ActivityResultContracts.GetContent(), uri -> {
            if (uri != null) readImage(uri);
        });

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
        Button scan = new Button(this); scan.setText(R.string.scan_camera); root.addView(scan);
        scan.setOnClickListener(v -> {
            if (!readingImage) camera.launch(new ScanOptions().setDesiredBarcodeFormats(ScanOptions.QR_CODE)
                .setPrompt(getString(R.string.scan_prompt)).setBeepEnabled(false).setOrientationLocked(false));
        });
        Button image = new Button(this); image.setText(R.string.scan_image); root.addView(image);
        image.setOnClickListener(v -> { if (!readingImage) gallery.launch("image/*"); });
        Button connect = new Button(this); connect.setText(R.string.text_4); root.addView(connect);
        status = new TextView(this); status.setText(R.string.text_5); root.addView(status);
        connect.setOnClickListener(v -> connect()); setContentView(root);
    }

    @SuppressWarnings("SetJavaScriptEnabled")
    private void connect() {
        String value = address.getText().toString().trim();
        String secret = token.getText().toString().trim();
        if (readingImage) return;
        try {
            origin = Pairing.manual(value, secret).origin;
        } catch (IllegalArgumentException e) { status.setText(R.string.invalid_pairing); return; }
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
    private void acceptPairing(String contents) {
        try {
            Pairing pairing = Pairing.parse(contents);
            address.setText(pairing.origin);
            token.setText(pairing.token);
            connect();
        } catch (IllegalArgumentException e) { status.setText(R.string.invalid_qr); }
    }

    private void readImage(Uri uri) {
        if (readingImage) return;
        readingImage = true;
        status.setText(R.string.reading_qr);
        imageWorker.execute(() -> {
            try {
                String contents = QrImage.read(getContentResolver(), uri);
                runOnUiThread(() -> {
                    readingImage = false;
                    if (!isDestroyed() && !isFinishing()) acceptPairing(contents);
                });
            } catch (Exception e) {
                runOnUiThread(() -> {
                    readingImage = false;
                    if (!isDestroyed() && !isFinishing()) status.setText(R.string.image_qr_failed);
                });
            }
        });
    }

    @Override protected void onPause() { if(web != null) web.onPause(); super.onPause(); }
    @Override protected void onResume() { super.onResume(); if(web != null) web.onResume(); }
    @Override protected void onDestroy() { if(web != null) { web.destroy(); web = null; } imageWorker.shutdownNow(); super.onDestroy(); }
}
