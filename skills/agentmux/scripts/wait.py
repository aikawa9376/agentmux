#!/usr/bin/env python3
"""Wait for future tmux output or an agentmux semantic status."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import time


def run(argv: list[str]) -> str:
    result = subprocess.run(argv, text=True, capture_output=True)
    if result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or f"command failed: {argv!r}")
    return result.stdout


def tmux_command(socket_name: str | None, *args: str) -> list[str]:
    command = ["tmux"]
    if socket_name:
        command.extend(["-L", socket_name])
    command.extend(args)
    return command


def agentmux_command(socket_name: str | None, *args: str) -> list[str]:
    command = ["agentmux"]
    if socket_name:
        command.extend(["--socket-name", socket_name])
    command.extend(args)
    return command


def pane_output(target: str, history_lines: int, socket_name: str | None) -> str:
    return run(
        tmux_command(
            socket_name,
            "capture-pane",
            "-pJ",
            "-S",
            f"-{history_lines}",
            "-t",
            target,
        )
    )


def matches(text: str, pattern: str, regex: bool) -> bool:
    return re.search(pattern, text, re.MULTILINE) is not None if regex else pattern in text


def wait_output(args: argparse.Namespace) -> int:
    baseline = pane_output(args.target, args.history_lines, args.socket_name)
    if args.include_existing and matches(baseline, args.match, args.regex):
        print(json.dumps({"matched": True, "target": args.target, "existing": True}))
        return 0

    deadline = time.monotonic() + args.timeout / 1000
    previous = baseline
    while time.monotonic() < deadline:
        time.sleep(args.interval / 1000)
        current = pane_output(args.target, args.history_lines, args.socket_name)
        if current == previous:
            continue
        if current.startswith(baseline):
            candidate = current[len(baseline) :]
        else:
            # Scrollback may have rolled over. Only inspect content that was not
            # present in the immediately preceding snapshot when possible.
            common = 0
            limit = min(len(previous), len(current))
            while common < limit and previous[common] == current[common]:
                common += 1
            candidate = current[common:]
        if matches(candidate, args.match, args.regex):
            print(json.dumps({"matched": True, "target": args.target, "existing": False}))
            return 0
        previous = current
    print(f"timed out waiting for output from {args.target}", file=sys.stderr)
    return 1


def resolve_pane(target: str, socket_name: str | None) -> dict:
    snapshot = json.loads(run(agentmux_command(socket_name, "list", "--json")))
    panes = snapshot.get("panes", [])
    exact = [
        pane
        for pane in panes
        if target
        in {
            pane.get("pane_id"),
            f"{pane.get('session_name')}:{pane.get('window_index')}.{pane.get('pane_index')}",
            pane.get("agent_name"),
        }
    ]
    if not exact:
        exact = [pane for pane in panes if pane.get("agent_kind") == target]
    if not exact:
        raise RuntimeError(f"no pane or agent matches {target!r}")
    if len(exact) > 1:
        ids = ", ".join(str(pane.get("pane_id")) for pane in exact)
        raise RuntimeError(f"target {target!r} is ambiguous: {ids}")
    return exact[0]


def wait_status(args: argparse.Namespace) -> int:
    deadline = time.monotonic() + args.timeout / 1000
    while True:
        pane = resolve_pane(args.target, args.socket_name)
        if pane.get("state") == args.status:
            print(
                json.dumps(
                    {
                        "matched": True,
                        "target": pane.get("pane_id"),
                        "state": pane.get("state"),
                        "source": pane.get("state_source"),
                    }
                )
            )
            return 0
        if time.monotonic() >= deadline:
            print(
                f"timed out waiting for {args.target} to become {args.status}; "
                f"last state was {pane.get('state')}",
                file=sys.stderr,
            )
            return 1
        time.sleep(args.interval / 1000)


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    root.add_argument("--socket-name", help="select a tmux socket by name")
    subcommands = root.add_subparsers(dest="command", required=True)

    output = subcommands.add_parser("output", help="wait for new tmux pane output")
    output.add_argument("target")
    output.add_argument("--match", required=True)
    output.add_argument("--regex", action="store_true")
    output.add_argument("--include-existing", action="store_true")
    output.add_argument("--timeout", type=int, default=30_000, help="milliseconds")
    output.add_argument("--interval", type=int, default=100, help="milliseconds")
    output.add_argument("--history-lines", type=int, default=400)
    output.set_defaults(handler=wait_output)

    status = subcommands.add_parser("status", help="wait for an agentmux state")
    status.add_argument("target")
    status.add_argument(
        "--status",
        required=True,
        choices=["blocked", "done", "working", "idle", "unknown", "dead", "pane"],
    )
    status.add_argument("--timeout", type=int, default=30_000, help="milliseconds")
    status.add_argument("--interval", type=int, default=200, help="milliseconds")
    status.set_defaults(handler=wait_status)
    return root


def main() -> int:
    args = parser().parse_args()
    try:
        return args.handler(args)
    except (RuntimeError, json.JSONDecodeError, re.error) as error:
        print(error, file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
