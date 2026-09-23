#!/usr/bin/env python3
"""Redact VPN acceptance credentials from a subprocess output stream."""

import json
import os
import sys


ERROR = "[acceptance] ERROR: browser output redaction configuration failed\n"


def load_secrets() -> list[str]:
    credentials_path = os.environ["VPN_LAB_CREDENTIALS_FILE"]
    password = os.environ["ADMIN_PASSWORD"]
    if not credentials_path or not password:
        raise ValueError

    with open(credentials_path, encoding="utf-8") as stream:
        credentials = json.load(stream)
    if not isinstance(credentials, dict):
        raise ValueError
    private_key = credentials.get("private_key")
    if not isinstance(private_key, str) or not private_key:
        raise ValueError

    variants = {password, private_key}
    for secret in tuple(variants):
        variants.add(json.dumps(secret, ensure_ascii=True)[1:-1])
        variants.add(json.dumps(secret, ensure_ascii=False)[1:-1])
    return sorted(variants, key=len, reverse=True)


def main() -> int:
    try:
        secrets = load_secrets()
    except (KeyError, OSError, UnicodeError, ValueError, TypeError, json.JSONDecodeError):
        sys.stderr.write(ERROR)
        sys.stderr.flush()
        return 2

    try:
        for line in sys.stdin:
            for secret in secrets:
                line = line.replace(secret, "[REDACTED]")
            sys.stdout.write(line)
            sys.stdout.flush()
    except BrokenPipeError:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
