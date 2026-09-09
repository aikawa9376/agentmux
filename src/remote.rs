//! Token-authenticated LAN mirror with explicit send and interrupt actions.
use crate::{model::Snapshot, tmux::Tmux};
use anyhow::{Context, Result, bail};
use std::{
    fs::File,
    io::{BufRead, BufReader, Read, Write},
    net::{IpAddr, SocketAddr, TcpListener, TcpStream},
    path::Path,
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

pub fn serve(
    tmux: Tmux,
    bind: SocketAddr,
    token_file: Option<&Path>,
    advertise_address: Option<IpAddr>,
    qr_svg: Option<&Path>,
) -> Result<()> {
    let token = match token_file {
        Some(path) => std::fs::read_to_string(path)?.trim().to_owned(),
        None => {
            let mut bytes = [0u8; 16];
            File::open("/dev/urandom")?.read_exact(&mut bytes)?;
            bytes.iter().map(|byte| format!("{byte:02x}")).collect()
        }
    };
    if token.len() < 32
        || !token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        bail!("token must contain at least 32 ASCII letters, digits, '-' or '_'");
    }
    let listener = TcpListener::bind(bind).context("could not bind mirror server")?;
    eprintln!(
        "Agentmux mirror: http://{} (use the PC's LAN IP on Android)",
        listener.local_addr()?
    );
    eprintln!("Pairing token: {token}");
    show_pairing(listener.local_addr()?, advertise_address, &token, qr_svg)?;
    let token = Arc::new(token);
    let (tx, rx) = mpsc::sync_channel::<TcpStream>(16);
    let rx = Arc::new(Mutex::new(rx));
    for _ in 0..4 {
        let (rx, token, tmux) = (rx.clone(), token.clone(), tmux.clone());
        std::thread::spawn(move || {
            loop {
                let stream = rx.lock().unwrap().recv();
                let Ok(mut stream) = stream else { break };
                let _ = handle(&mut stream, &tmux, &token);
            }
        });
    }
    for stream in listener.incoming() {
        let stream = stream?;
        // Bound queued connections as well as worker count.
        let _ = tx.try_send(stream);
    }
    Ok(())
}

fn pairing_url(address: SocketAddr, token: &str) -> String {
    format!("http://{address}/#token={token}")
}

fn pairing_addresses(bind: SocketAddr, advertised: Option<IpAddr>) -> Result<Vec<SocketAddr>> {
    let ips = if let Some(ip) = advertised {
        if ip.is_unspecified() || ip.is_loopback() || ip.is_multicast() {
            bail!("--advertise-address must be a reachable LAN IP");
        }
        if !bind.ip().is_unspecified() && bind.ip() != ip {
            bail!("--advertise-address must match a concrete --bind IP");
        }
        if bind.is_ipv4() != ip.is_ipv4() {
            bail!("--advertise-address and --bind must use the same IP family");
        }
        vec![ip]
    } else if bind.ip().is_unspecified() {
        let mut interfaces: Vec<_> = if_addrs::get_if_addrs()?
            .into_iter()
            .filter(|interface| { let ip = interface.ip(); !ip.is_loopback() && !ip.is_unspecified() && !ip.is_multicast() })
            .filter(|interface| interface.ip().is_ipv4() == bind.is_ipv4())
            // Link-local IPv6 requires a phone-specific interface scope, so cannot pair by QR.
            .filter(|interface| !matches!(interface.ip(), IpAddr::V6(v6) if v6.segments()[0] & 0xffc0 == 0xfe80))
            .collect();
        let preferred = default_route_interface();
        interfaces.sort_by_key(|interface| {
            (
                interface_rank(&interface.name, preferred.as_deref()),
                interface.ip(),
            )
        });
        interfaces
            .first()
            .map(|interface| vec![interface.ip()])
            .unwrap_or_default()
    } else if bind.ip().is_loopback() {
        vec![]
    } else {
        vec![bind.ip()]
    };
    Ok(ips
        .into_iter()
        .map(|ip| SocketAddr::new(ip, bind.port()))
        .collect())
}

fn default_route_interface() -> Option<String> {
    // Linux route table, no external network request or dependency on `ip`.
    let routes = std::fs::read_to_string("/proc/net/route").ok()?;
    routes
        .lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            if fields.len() < 8 || fields[1] != "00000000" || fields[7] != "00000000" {
                return None;
            }
            let flags = u32::from_str_radix(fields[3], 16).ok()?;
            if flags & 1 == 0 {
                return None;
            }
            Some((fields[6].parse::<u32>().ok()?, fields[0].to_owned()))
        })
        .min()
        .map(|(_, name)| name)
}

fn interface_rank(name: &str, preferred: Option<&str>) -> u8 {
    let virtual_interface = [
        "docker",
        "veth",
        "br-",
        "virbr",
        "tun",
        "tap",
        "wg",
        "tailscale",
        "zt",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix));
    if virtual_interface {
        3
    } else if preferred == Some(name) {
        0
    } else if name.starts_with("en") || name.starts_with("eth") || name.starts_with("wl") {
        1
    } else {
        2
    }
}

fn show_pairing(
    bind: SocketAddr,
    advertised: Option<IpAddr>,
    token: &str,
    svg_path: Option<&Path>,
) -> Result<()> {
    use qrcode::{
        QrCode,
        render::{svg, unicode},
    };
    let addresses = pairing_addresses(bind, advertised)?;
    if svg_path.is_some() && addresses.len() != 1 {
        bail!(
            "SVG export needs one LAN address; specify --bind 0.0.0.0:9876 --advertise-address <LAN IP>"
        );
    }
    if addresses.is_empty() {
        eprintln!("LAN pairing QR unavailable. Use --bind 0.0.0.0:9876 to listen on the LAN.");
    }
    for address in addresses {
        let url = pairing_url(address, token);
        let code = QrCode::new(url.as_bytes()).context("pairing URL is too long for a QR code")?;
        let qr = code.render::<unicode::Dense1x2>().build();
        eprintln!("\nScan in Agentmux Mirror: http://{address}");
        // Force contrast independent of the user's terminal color scheme.
        for line in qr.lines() {
            eprintln!("\x1b[30;47m{line}\x1b[0m");
        }
        if let Some(path) = svg_path {
            use std::os::unix::fs::OpenOptionsExt;
            let svg = code.render::<svg::Color>().min_dimensions(512, 512).build();
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)
                .context("could not create private QR SVG (file must not already exist)")?;
            file.write_all(svg.as_bytes())?;
            eprintln!(
                "Pairing QR saved to {} (contains your access token)",
                path.display()
            );
        }
    }
    Ok(())
}

fn handle(stream: &mut TcpStream, tmux: &Tmux, token: &str) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let mut reader = BufReader::new(&mut *stream);
    let mut header = String::new();
    loop {
        let start = header.len();
        if (&mut reader)
            .take((8193 - start) as u64)
            .read_line(&mut header)?
            == 0
            || header.len() > 8192
        {
            return respond(stream, 400, "text/plain", b"Invalid request");
        }
        if &header[start..] == "\r\n" {
            break;
        }
    }
    let length = match body_length(&header) {
        Ok(length) => length,
        Err(_) => return respond(stream, 400, "text/plain", b"Invalid Content-Length"),
    };
    if length > 16384 {
        return respond(stream, 413, "text/plain", b"Request too large");
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    let (status, mime, body) = route_body(&header, &body, tmux, token);
    respond(stream, status, mime, &body)
}

fn body_length(header: &str) -> Result<usize> {
    let mut length = None;
    for (name, value) in header.lines().filter_map(|line| line.split_once(':')) {
        if name.eq_ignore_ascii_case("transfer-encoding") {
            bail!("chunked requests unsupported");
        }
        if name.eq_ignore_ascii_case("content-length") {
            if length.is_some() {
                bail!("duplicate length");
            }
            length = Some(value.trim().parse::<usize>()?);
        }
    }
    Ok(length.unwrap_or(0))
}

#[cfg(test)]
fn route(header: &str, tmux: &Tmux, token: &str) -> (u16, &'static str, Vec<u8>) {
    route_body(header, &[], tmux, token)
}

fn route_body(header: &str, body: &[u8], tmux: &Tmux, token: &str) -> (u16, &'static str, Vec<u8>) {
    let mut lines = header.split("\r\n");
    let parts: Vec<_> = lines
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .collect();
    let plain = |status, text: &str| {
        (
            status,
            "text/plain; charset=utf-8",
            text.as_bytes().to_vec(),
        )
    };
    if parts.len() != 3 || !matches!(parts[0], "GET" | "POST") {
        return plain(405, "Method not allowed");
    }
    match (parts[0], parts[1]) {
        ("GET", "/") => {
            return (
                200,
                "text/html; charset=utf-8",
                include_bytes!("../web/index.html").to_vec(),
            );
        }
        ("GET", "/mirror.js") => {
            return (
                200,
                "text/javascript; charset=utf-8",
                include_bytes!("../web/mirror.js").to_vec(),
            );
        }
        _ => {}
    }
    let authorized = lines
        .filter_map(|line| line.split_once(':'))
        .any(|(name, value)| {
            name.eq_ignore_ascii_case("authorization") && value.trim() == format!("Bearer {token}")
        });
    if !authorized {
        return plain(401, "Pairing token required");
    }
    if parts[0] == "POST" {
        if parts[1] != "/api/action" {
            return plain(405, "Method not allowed");
        }
        let json_content =
            header
                .lines()
                .filter_map(|line| line.split_once(':'))
                .any(|(name, value)| {
                    name.eq_ignore_ascii_case("content-type")
                        && value.trim().eq_ignore_ascii_case("application/json")
                });
        if !json_content {
            return plain(415, "JSON required");
        }
        let Ok(request) = serde_json::from_slice::<crate::control::Request>(body) else {
            return plain(400, "Invalid action");
        };
        if !valid_pane_id(&request.pane) {
            return plain(400, "Invalid pane");
        }
        return match crate::control::execute(tmux, request) {
            Ok(()) => (200, "application/json", b"{\"accepted\":true}".to_vec()),
            Err(_) => plain(
                409,
                "Action unavailable or unconfirmed; refresh before retrying",
            ),
        };
    }
    let result: Result<Vec<u8>> = (|| match parts[1] {
        "/api/agents" => {
            let mut snapshot = tmux.snapshot()?;
            snapshot.panes.retain(|pane| pane.is_agent());
            snapshot.spaces = Snapshot::build_spaces(&snapshot.panes);
            let controls: std::collections::BTreeMap<_, _> = snapshot
                .panes
                .iter()
                .filter_map(|pane| {
                    crate::control::capabilities(tmux, pane)
                        .ok()
                        .map(|info| (pane.pane_id.clone(), info))
                })
                .collect();
            let mut response = serde_json::to_value(&snapshot)?;
            response["controls"] = serde_json::to_value(controls)?;
            Ok(serde_json::to_vec(&response)?)
        }
        path if path.starts_with("/api/view/") => {
            let id = &path[10..];
            if !valid_pane_id(id) {
                bail!("invalid pane id");
            }
            let snapshot = tmux.snapshot()?;
            if !snapshot.pane(id).is_some_and(|pane| pane.is_agent()) {
                bail!("agent unavailable");
            }
            Ok(serde_json::to_vec(&tmux.pane_view(id)?)?)
        }
        _ => bail!("unknown endpoint"),
    })();
    match result {
        Ok(body) => (200, "application/json", body),
        // Do not expose local transcript paths or tmux diagnostics to clients.
        Err(_) => plain(404, "Agent or preview unavailable"),
    }
}

fn valid_pane_id(id: &str) -> bool {
    id.starts_with('%') && id.len() > 1 && id[1..].bytes().all(|b| b.is_ascii_digit())
}

fn respond(stream: &mut TcpStream, status: u16, mime: &str, body: &[u8]) -> Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status} {}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'; frame-ancestors 'none'\r\n\r\n",
        if status == 200 { "OK" } else { "Error" },
        body.len()
    )?;
    stream.write_all(body)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn api_requires_exact_bearer_and_rejects_unknown_actions() {
        let tmux = Tmux::new(Some("unused".into()));
        assert_eq!(
            route("GET /api/agents HTTP/1.1\r\n\r\n", &tmux, "secret").0,
            401
        );
        assert_eq!(
            route(
                "GET /api/agents HTTP/1.1\r\nAuthorization: Bearer wrong\r\n\r\n",
                &tmux,
                "secret"
            )
            .0,
            401
        );
        assert_eq!(
            route(
                "POST /api/send HTTP/1.1\r\nAuthorization: Bearer secret\r\n\r\n",
                &tmux,
                "secret"
            )
            .0,
            405
        );
        assert_eq!(
            route(
                "GET /api/view/../../etc/passwd HTTP/1.1\r\nAuthorization: Bearer secret\r\n\r\n",
                &tmux,
                "secret"
            )
            .0,
            404
        );
        assert_eq!(route("GET / HTTP/1.1\r\n\r\n", &tmux, "secret").0, 200);
    }
    #[test]
    fn prefer_default_physical_lan_and_compact_qr() {
        assert!(interface_rank("wlan0", Some("wlan0")) < interface_rank("eth0", Some("wlan0")));
        assert!(interface_rank("wlan0", Some("tun0")) < interface_rank("tun0", Some("tun0")));
        assert!(interface_rank("enp5s0", None) < interface_rank("docker0", None));
        let address = "192.168.1.20:9876".parse().unwrap();
        let old = qrcode::QrCode::new(pairing_url(address, &"a".repeat(64))).unwrap();
        let compact = qrcode::QrCode::new(pairing_url(address, &"a".repeat(32))).unwrap();
        assert!(compact.width() < old.width());
    }

    #[test]
    fn reject_ambiguous_http_framing_and_invalid_actions() {
        assert!(
            body_length("POST /api/action HTTP/1.1\r\nContent-Length: 1\r\nContent-Length: 2\r\n")
                .is_err()
        );
        assert!(body_length("Transfer-Encoding: chunked\r\n").is_err());
        assert!(body_length("Content-Length: -1\r\n").is_err());
        assert_eq!(body_length("Content-Length: 42\r\n").unwrap(), 42);
        let tmux = Tmux::new(Some("unused".into()));
        let header = "POST /api/action HTTP/1.1\r\nAuthorization: Bearer secret\r\nContent-Type: application/json\r\n\r\n";
        assert_eq!(route_body(header, b"{}", &tmux, "secret").0, 400);
        assert_eq!(
            route_body(
                header,
                br#"{"pane":"%1","binding":"x","action":"kill"}"#,
                &tmux,
                "secret"
            )
            .0,
            400
        );
        assert_eq!(route_body(header, b"{}", &tmux, "wrong").0, 401);
    }

    #[test]
    fn pairing_uses_actual_port_and_fragment_token() {
        assert_eq!(
            pairing_url("192.168.1.20:1234".parse().unwrap(), "abc"),
            "http://192.168.1.20:1234/#token=abc"
        );
        assert_eq!(
            pairing_url("[fd00::1]:9876".parse().unwrap(), "abc"),
            "http://[fd00::1]:9876/#token=abc"
        );
        assert!(
            pairing_addresses("127.0.0.1:9876".parse().unwrap(), None)
                .unwrap()
                .is_empty()
        );
        assert!(
            pairing_addresses(
                "0.0.0.0:9876".parse().unwrap(),
                Some("0.0.0.0".parse().unwrap())
            )
            .is_err()
        );
        assert!(
            pairing_addresses(
                "127.0.0.1:9876".parse().unwrap(),
                Some("192.168.1.20".parse().unwrap())
            )
            .is_err()
        );
        assert_eq!(
            pairing_addresses(
                "0.0.0.0:1234".parse().unwrap(),
                Some("192.168.1.20".parse().unwrap())
            )
            .unwrap(),
            vec!["192.168.1.20:1234".parse::<SocketAddr>().unwrap()]
        );
    }

    #[test]
    fn targets_are_literal_pane_ids() {
        assert!(valid_pane_id("%12"));
        for id in ["%", "12", "%1;kill-server", "%1/other", "%a"] {
            assert!(!valid_pane_id(id));
        }
    }
}
