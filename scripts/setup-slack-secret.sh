#!/usr/bin/env bash
set -euo pipefail

name=lotto-daemon-slack
if podman secret inspect "$name" >/dev/null 2>&1; then
    printf '%s\n' 'Slack secret already exists; not replacing it.'
    exit 0
fi

python3 - "$name" <<'PY'
import re
import subprocess
import sys
from pathlib import Path
from urllib.parse import urlsplit

try:
    env_lines = Path('/etc/env/slack/.env').read_text(encoding='utf-8').splitlines()
except (OSError, UnicodeError):
    raise SystemExit('Cannot read the Slack environment file') from None

matches = [m[1].strip() for line in env_lines
           if (m := re.fullmatch(r'\s*(?:export\s+)?WEBHOOK_URL_DOT_CHANNEL\s*=\s*(.*?)\s*', line))]
if len(matches) != 1:
    raise SystemExit('Expected one WEBHOOK_URL_DOT_CHANNEL entry')
value = matches[0]
if len(value) >= 2 and value[0] == value[-1] and value[0] in "\"'":
    value = value[1:-1]
try:
    url = urlsplit(value)
    if (url.scheme != 'https' or url.netloc not in ('hooks.slack.com', 'hooks.slack-gov.com')
            or not re.fullmatch(r'/services/[A-Za-z0-9_-]+/[A-Za-z0-9_-]+/[A-Za-z0-9_-]+', url.path)
            or url.query or url.fragment):
        raise ValueError
except ValueError:
    raise SystemExit('Invalid Slack Incoming Webhook URL') from None
subprocess.run(['podman', 'secret', 'create', sys.argv[1], '-'], input=value,
               text=True, stdout=subprocess.DEVNULL, check=True)
PY
printf '%s\n' 'Podman Slack secret is ready.'
