package dev.agentmux.mirror;

import android.content.ContentResolver;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.net.Uri;
import com.google.zxing.BinaryBitmap;
import com.google.zxing.DecodeHintType;
import com.google.zxing.RGBLuminanceSource;
import com.google.zxing.Result;
import com.google.zxing.common.HybridBinarizer;
import com.google.zxing.multi.qrcode.QRCodeMultiReader;
import java.io.InputStream;
import java.util.EnumMap;
import java.util.LinkedHashSet;
import java.util.Map;
import java.util.Set;

/** Decode locally, bounding pixel allocation even for large gallery images. */
final class QrImage {
    static String read(ContentResolver resolver, Uri uri) throws Exception {
        BitmapFactory.Options options = new BitmapFactory.Options();
        options.inJustDecodeBounds = true;
        try (InputStream stream = resolver.openInputStream(uri)) {
            BitmapFactory.decodeStream(stream, null, options);
        }
        if (options.outWidth <= 0 || options.outHeight <= 0) throw new IllegalArgumentException("Invalid image");
        options.inSampleSize = 1;
        while (options.outWidth / options.inSampleSize > 2048 || options.outHeight / options.inSampleSize > 2048) {
            options.inSampleSize *= 2;
        }
        options.inJustDecodeBounds = false;
        options.inPreferredConfig = Bitmap.Config.ARGB_8888;
        Bitmap bitmap;
        try (InputStream stream = resolver.openInputStream(uri)) {
            bitmap = BitmapFactory.decodeStream(stream, null, options);
        }
        if (bitmap == null) throw new IllegalArgumentException("Invalid image");
        try {
            int width = bitmap.getWidth(), height = bitmap.getHeight();
            int[] pixels = new int[width * height];
            bitmap.getPixels(pixels, 0, width, 0, 0, width, height);
            return decode(width, height, pixels);
        } finally { bitmap.recycle(); }
    }

    static String decode(int width, int height, int[] pixels) throws Exception {
        BinaryBitmap source = new BinaryBitmap(new HybridBinarizer(new RGBLuminanceSource(width, height, pixels)));
        Map<DecodeHintType, Object> hints = new EnumMap<>(DecodeHintType.class);
        hints.put(DecodeHintType.TRY_HARDER, true);
        Result[] results = new QRCodeMultiReader().decodeMultiple(source, hints);
        Set<String> valid = new LinkedHashSet<>();
        for (Result result : results) {
            try { Pairing.parse(result.getText()); valid.add(result.getText()); }
            catch (IllegalArgumentException ignored) { /* Other QR formats are not connections. */ }
        }
        if (valid.size() != 1) throw new IllegalArgumentException("Select an image with one Agentmux QR");
        return valid.iterator().next();
    }
}
