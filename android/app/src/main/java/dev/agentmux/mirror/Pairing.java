package dev.agentmux.mirror;

import java.net.URI;

/** Shared validation for manual connections and scanned pairing URLs. */
final class Pairing {
    final String origin;
    final String token;

    private Pairing(String origin, String token) {
        this.origin = origin;
        this.token = token;
    }

    static Pairing manual(String address, String token) {
        try {
            URI uri = new URI(address.trim());
            if (!("http".equals(uri.getScheme()) || "https".equals(uri.getScheme()))
                    || uri.getHost() == null || uri.getUserInfo() != null
                    || uri.getQuery() != null || uri.getFragment() != null
                    || !(uri.getPath().isEmpty() || uri.getPath().equals("/"))
                    || uri.getPort() == 0 || uri.getPort() > 65535
                    || !token.matches("[A-Za-z0-9_-]{32,512}")) {
                throw new IllegalArgumentException("Invalid pairing address or token");
            }
            return new Pairing(uri.getScheme() + "://" + uri.getRawAuthority(), token);
        } catch (java.net.URISyntaxException e) {
            throw new IllegalArgumentException("Invalid pairing address", e);
        }
    }

    static Pairing parse(String value) {
        if (value == null || value.length() > 2048) throw new IllegalArgumentException("Invalid QR");
        int fragment = value.indexOf('#');
        if (fragment < 0 || !value.substring(fragment).startsWith("#token=")) {
            throw new IllegalArgumentException("Not an Agentmux pairing QR");
        }
        return manual(value.substring(0, fragment), value.substring(fragment + 7));
    }
}
