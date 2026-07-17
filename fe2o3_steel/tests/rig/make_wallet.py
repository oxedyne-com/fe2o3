#!/usr/bin/env python3
"""Create a Steel wallet in the rig.

Wallet creation reads a choice and an admin name from stdin, then prompts twice
for a passphrase through crossterm's raw mode -- which needs a real terminal, so
a pipe will not do. This gives it a pty. In raw mode Enter arrives as a carriage
return, not a newline; a newline is not a submit and the prompt simply waits.

The passphrase is a throwaway for a wallet that lives for thirty seconds and
holds nothing.
"""

import os
import pty
import re
import select
import sys
import time

RIG  = os.environ.get("RIG_DIR") or sys.exit("RIG_DIR is not set")
PASS = "rig-test-passphrase-not-a-secret"
NAME = "rig"

pid, fd = pty.fork()
if pid == 0:
    os.chdir(RIG)
    os.environ["TERM"] = "xterm"
    os.execv(RIG + "/steel", [RIG + "/steel"])

log  = []
sent = set()
deadline = time.time() + 60

def feed(what, tag):
    if tag in sent:
        return
    sent.add(tag)
    os.write(fd, what.encode())

while time.time() < deadline:
    r, _, _ = select.select([fd], [], [], 0.5)
    if not r:
        continue
    try:
        chunk = os.read(fd, 4096).decode("utf-8", "replace")
    except OSError:
        break
    if not chunk:
        break
    log.append(chunk)
    sys.stdout.write(chunk)
    sys.stdout.flush()
    blob = "".join(log)

    # The menu: choose to create a new wallet.
    if re.search(r"2\s*[.):]", blob) and "choice" not in sent:
        sent.add("choice")
        time.sleep(0.3)
        os.write(fd, b"2\n")
        continue
    # The admin's name.
    if re.search(r"(name|user)", blob, re.I) and "choice" in sent and "name" not in sent:
        sent.add("name")
        time.sleep(0.3)
        os.write(fd, (NAME + "\n").encode())
        continue
    # Two passphrase prompts. These are read in crossterm raw mode, where Enter
    # arrives as a carriage return -- a newline is not a submit and the prompt
    # simply waits.
    if blob.count("Enter a passphrase") >= 1 and "p1" not in sent:
        sent.add("p1")
        time.sleep(0.5)
        os.write(fd, (PASS + "\r").encode())
        continue
    if blob.count("Re-enter") >= 1 and "p2" not in sent:
        sent.add("p2")
        time.sleep(0.5)
        os.write(fd, (PASS + "\r").encode())
        continue
    # Done: leave the shell.
    if ("p2" in sent) and re.search(r"(>|\$|shell|Steel)", blob) and "exit" not in sent:
        sent.add("exit")
        time.sleep(1.0)
        os.write(fd, b"exit\n")
        break

time.sleep(1.5)
os.close(fd)
print("\n--- steps taken:", sorted(sent))
print("--- wallet.jdat:", "PRESENT" if os.path.exists(RIG + "/wallet.jdat") else "ABSENT")
