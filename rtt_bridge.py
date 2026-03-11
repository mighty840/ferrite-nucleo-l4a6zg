#!/usr/bin/env python3
"""RTT-to-Server Bridge.

Captures ferrite binary chunks from the Nucleo board via probe-rs RTT output
and POSTs them to the ferrite-server's /ingest/chunks endpoint.

Usage:
    python3 rtt_bridge.py [--server URL] [--device-id ID]

Requires:
    pip install requests
    probe-rs (https://probe.rs)
"""

import argparse
import os
import re
import subprocess
import sys

import requests

CHUNK_RE = re.compile(r"CHUNK:\[([0-9a-f, ]+)\]")

ELF_PATH = "target/thumbv7em-none-eabihf/release/ferrite-nucleo-l4a6zg"
CHIP = "STM32L4A6ZGTx"


def parse_chunk_hex(hex_str: str) -> bytes:
    """Parse comma-separated hex like 'ec, 1, 1, ...' into bytes."""
    return bytes(int(b.strip(), 16) for b in hex_str.split(","))


def post_chunk(session: requests.Session, server: str, device_id: str, chunk: bytes) -> bool:
    """POST a binary chunk to the server ingest endpoint."""
    try:
        resp = session.post(
            f"{server}/ingest/chunks",
            data=chunk,
            headers={
                "Content-Type": "application/octet-stream",
                "X-Device-Id": device_id,
            },
            timeout=5,
        )
        return resp.status_code in (200, 207)
    except requests.RequestException as e:
        print(f"  POST error: {e}", file=sys.stderr, flush=True)
        return False


def main():
    parser = argparse.ArgumentParser(description="RTT-to-Server bridge for ferrite chunks")
    parser.add_argument("--server", default="http://localhost:4000", help="ferrite-server URL")
    parser.add_argument("--device-id", default="nucleo-l4a6zg-01", help="fallback device ID header")
    parser.add_argument("--user", default="admin", help="basic auth username")
    parser.add_argument("--password", default="admin", help="basic auth password")
    args = parser.parse_args()

    if not os.path.exists(ELF_PATH):
        print(f"ELF not found at {ELF_PATH}. Run: cargo build --release", file=sys.stderr)
        sys.exit(1)

    session = requests.Session()
    session.auth = (args.user, args.password)

    print(f"RTT Bridge: {CHIP} -> {args.server}")
    print("Press Ctrl+C to stop\n", flush=True)

    cmd = ["probe-rs", "run", "--chip", CHIP, ELF_PATH]
    proc = subprocess.Popen(
        cmd,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        bufsize=1,
        env={**os.environ, "PYTHONUNBUFFERED": "1"},
    )

    sent = 0
    failed = 0

    try:
        for line in proc.stdout:
            line = line.strip()

            m = CHUNK_RE.search(line)
            if m:
                chunk = parse_chunk_hex(m.group(1))
                ok = post_chunk(session, args.server, args.device_id, chunk)
                if ok:
                    sent += 1
                else:
                    failed += 1
                tag = "OK" if ok else "FAIL"
                seq = chunk[6] | (chunk[7] << 8) if len(chunk) > 7 else 0
                print(f"  [{sent}/{sent + failed}] {tag} seq={seq} len={len(chunk)}B", flush=True)
            elif "boot ok" in line:
                print(f"  {line}", flush=True)
            elif "iteration" in line:
                msg = line.split("]")[-1].strip() if "]" in line else line
                print(f"  board: {msg}", flush=True)
            elif "Finished in" in line:
                print(f"  Flashed: {line}", flush=True)

    except KeyboardInterrupt:
        pass
    finally:
        print(f"\nSent {sent} chunks, {failed} failed.", flush=True)
        proc.terminate()
        proc.wait()


if __name__ == "__main__":
    main()
