#!/usr/bin/env python3
import json
import socket
import struct
import sys
import time


def send(sock, req):
    data = json.dumps(req).encode()
    sock.sendall(struct.pack("<I", len(data)) + data)
    raw = sock.recv(4)
    if len(raw) < 4:
        raise ConnectionError("Worker closed connection")
    length = struct.unpack("<I", raw)[0]
    data = b""
    while len(data) < length:
        chunk = sock.recv(length - len(data))
        if not chunk:
            raise ConnectionError("Worker closed mid-response")
        data += chunk
    return json.loads(data.decode("utf-8"))


def main():
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    sock.settimeout(120)
    try:
        sock.connect("/tmp/mavis_worker.sock")
    except FileNotFoundError:
        print("FAIL: Socket not found. Is the worker running?")
        sys.exit(1)

    print("=" * 50)
    print("1. HEALTH")
    r = send(sock, {"type": "health"})
    print(f"   RAW: {json.dumps(r, indent=2)}")

    print("=" * 50)
    print("2. CHAT (direct format)")
    t0 = time.time()
    r = send(
        sock,
        {
            "type": "chat",
            "payload": {
                "messages": [{"role": "user", "content": "Say 'MAVIS online' and nothing else."}],
                "max_tokens": 32,
                "temperature": 0.1,
            },
        },
    )
    t1 = time.time()
    print(f"   Time: {t1 - t0:.2f}s")
    print(f"   RAW: {json.dumps(r, indent=2)}")

    print("=" * 50)
    print("3. CHAT (WorkerRequest event format)")
    t0 = time.time()
    r = send(
        sock,
        {
            "type": "WorkerRequest",
            "payload": {
                "request_type": "chat",
                "messages": [
                    {"role": "user", "content": "Say 'event format OK' and nothing else."}
                ],
                "max_tokens": 32,
                "temperature": 0.1,
            },
        },
    )
    t1 = time.time()
    print(f"   Time: {t1 - t0:.2f}s")
    print(f"   RAW: {json.dumps(r, indent=2)}")

    print("=" * 50)
    print("4. MEMORY")
    r = send(sock, {"type": "memory"})
    print(f"   RAW: {json.dumps(r, indent=2)}")

    print("=" * 50)
    print("5. UNLOAD")
    r = send(sock, {"type": "unload"})
    print(f"   RAW: {json.dumps(r, indent=2)}")


if __name__ == "__main__":
    main()
