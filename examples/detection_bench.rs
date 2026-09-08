//! Run with `cargo run --release --example detection_bench -- 10000`.
use agentmux::detect::screen_status_with_title;
use std::{hint::black_box, time::Instant};

fn main() {
    let iterations: usize = std::env::args()
        .nth(1)
        .unwrap_or("10000".into())
        .parse()
        .unwrap();
    let fixtures = [
        (
            "codex",
            "› inspect this project\n• Working (2s · esc to interrupt)",
            "⠋ project",
        ),
        ("claude", "Run a dynamic workflow? Esc to cancel", "project"),
        ("github-copilot", "Thinking\nEsc again to cancel", ""),
        ("unknown-agent", "plain shell output", ""),
    ];
    for (kind, screen, title) in fixtures {
        black_box(screen_status_with_title(kind, screen, title));
    }
    let start = Instant::now();
    for _ in 0..iterations {
        for (kind, screen, title) in fixtures {
            black_box(screen_status_with_title(
                black_box(kind),
                black_box(screen),
                black_box(title),
            ));
        }
    }
    println!(
        "{} detections in {:?}",
        iterations * fixtures.len(),
        start.elapsed()
    );
}
