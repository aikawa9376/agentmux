//! Read-only LAN mirror. The token authorizes access to agent screens only.
use crate::{model::Snapshot, tmux::Tmux};
use anyhow::{Context, Result, bail};
use std::{
    fs::File,
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::Path,
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

pub fn serve(tmux: Tmux, bind: SocketAddr, token_file: Option<&Path>) -> Result<()> {
    let token = match token_file {
        Some(path) => std::fs::read_to_string(path)?.trim().to_owned(),
        None => {
            let mut bytes = [0u8; 32];
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

fn handle(stream: &mut TcpStream, tmux: &Tmux, token: &str) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let mut reader = BufReader::new((&mut *stream).take(8193));
    let mut header = String::new();
    loop {
        let start = header.len();
        if reader.read_line(&mut header)? == 0 || header.len() > 8192 {
            return respond(stream, 400, "text/plain", b"Invalid request");
        }
        if &header[start..] == "\r\n" {
            break;
        }
    }
    let (status, mime, body) = route(&header, tmux, token);
    respond(stream, status, mime, &body)
}

fn route(header: &str, tmux: &Tmux, token: &str) -> (u16, &'static str, Vec<u8>) {
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
    if parts.len() != 3 || parts[0] != "GET" {
        return plain(405, "GET only");
    }
    match parts[1] {
        "/" => {
            return (
                200,
                "text/html; charset=utf-8",
                include_bytes!("../web/index.html").to_vec(),
            );
        }
        "/mirror.js" => {
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
    let result: Result<Vec<u8>> = (|| match parts[1] {
        "/api/agents" => {
            let mut snapshot = tmux.snapshot()?;
            snapshot.panes.retain(|pane| pane.is_agent());
            snapshot.spaces = Snapshot::build_spaces(&snapshot.panes);
            Ok(serde_json::to_vec(&snapshot)?)
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
    fn api_requires_exact_bearer_and_never_mutates() {
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
    fn targets_are_literal_pane_ids() {
        assert!(valid_pane_id("%12"));
        for id in ["%", "12", "%1;kill-server", "%1/other", "%a"] {
            assert!(!valid_pane_id(id));
        }
    }
}
