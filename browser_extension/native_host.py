#!/usr/bin/env python3
"""Native messaging host: browser extension -> MAVIS's browser socket.

Spawned by the browser itself when the extension calls connectNative() —
not run directly. Reads length-prefixed JSON from stdin (the native
messaging wire protocol, identical on Chrome and Firefox), forwards
{"url", "title"} to mavis_core as one line-delimited JSON write.
"""

import json
import socket
import struct
import sys

MAVIS_BROWSER_SOCKET = "/tmp/mavis_browser.sock"


def read_message():
    raw_length = sys.stdin.buffer.read(4)
    if len(raw_length) == 0:
        return None
    length = struct.unpack("<I", raw_length)[0]
    return json.loads(sys.stdin.buffer.read(length).decode("utf-8"))


def forward_to_mavis(url, title):
    # Fails silently if MAVIS isn't running — never crash the browser over this.
    try:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
            s.settimeout(1)
            s.connect(MAVIS_BROWSER_SOCKET)
            s.sendall((json.dumps({"url": url, "title": title}) + "\n").encode("utf-8"))
    except OSError:
        pass


def main():
    while True:
        message = read_message()
        if message is None:
            break
        url = message.get("url", "")
        title = message.get("title", "")
        if url:
            forward_to_mavis(url, title)


if __name__ == "__main__":
    main()
