package dev.agentmux.mirror;

import org.junit.Test;
import static org.junit.Assert.*;
import com.google.zxing.BarcodeFormat;
import com.google.zxing.MultiFormatWriter;
import com.google.zxing.common.BitMatrix;

public final class PairingTest {
    private static final String TOKEN = "0123456789abcdef0123456789abcdef";
    private static final String URL = "http://192.168.1.20:9876/#token=" + TOKEN;

    @Test public void acceptsLanAndIpv6Pairing() {
        Pairing result = Pairing.parse(URL);
        assertEquals("http://192.168.1.20:9876", result.origin);
        assertEquals(TOKEN, result.token);
        assertEquals("http://[fd00::1]:9876", Pairing.parse("http://[fd00::1]:9876/#token=" + TOKEN).origin);
        assertEquals("https://mirror.local", Pairing.manual("https://mirror.local/", TOKEN).origin);
    }

    @Test public void rejectsUnrelatedOrAmbiguousQrPayloads() {
        for (String value : new String[] {null, "plain text", "file:///tmp/a#token=" + TOKEN,
            "javascript:alert(1)", "http://host/?token=" + TOKEN, "http://user@host/#token=" + TOKEN,
            "http://host/other#token=" + TOKEN, "http://host/#token=short",
            URL + "&token=" + TOKEN, URL + "#other", "http://host:99999/#token=" + TOKEN}) {
            assertThrows(IllegalArgumentException.class, () -> Pairing.parse(value));
        }
    }

    @Test public void decodesQrPixelsAndRejectsNonPairingQr() throws Exception {
        assertEquals(URL, QrImage.decode(400, 400, pixels(URL)));
        assertThrows(IllegalArgumentException.class, () -> QrImage.decode(400, 400, pixels("https://example.com")));
    }

    private static int[] pixels(String value) throws Exception {
        BitMatrix matrix = new MultiFormatWriter().encode(value, BarcodeFormat.QR_CODE, 400, 400);
        int[] pixels = new int[400 * 400];
        for (int y = 0; y < 400; y++) for (int x = 0; x < 400; x++) pixels[y * 400 + x] = matrix.get(x, y) ? 0xff000000 : 0xffffffff;
        return pixels;
    }
}
