#!/usr/bin/env bash

set -euo pipefail

BOSSAC_VERSION="${BOSSAC_VERSION:-1.9.1}"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
dest_dir="${repo_root}/tools/linux"

mkdir -p "${dest_dir}"

tmpdir="$(mktemp -d)"
trap 'rm -rf "${tmpdir}"' EXIT

if [[ -n "${BOSSAC_URL:-}" ]]; then
  bossac_url="${BOSSAC_URL}"
else
  # Resolve the correct asset URL via GitHub API to avoid hard-coding
  # release tag / asset naming conventions.
  release_json="${tmpdir}/release.json"
  api_base="https://api.github.com/repos/shumatech/BOSSA/releases/tags"

  if curl -fsSL "${api_base}/${BOSSAC_VERSION}" -o "${release_json}"; then
    :
  elif curl -fsSL "${api_base}/v${BOSSAC_VERSION}" -o "${release_json}"; then
    :
  else
    echo "ERROR: Unable to resolve BOSSA release for tag '${BOSSAC_VERSION}' (or 'v${BOSSAC_VERSION}')." >&2
    exit 1
  fi

  bossac_url="$(
    python3 - "${release_json}" "${BOSSAC_VERSION}" << 'PY'
import json
import re
import sys

path = sys.argv[1]
version = sys.argv[2]

with open(path, "r", encoding="utf-8") as f:
    release = json.load(f)

assets = release.get("assets", []) or []

def pick(patterns):
    for pat in patterns:
        rx = re.compile(pat, re.IGNORECASE)
        for asset in assets:
            name = asset.get("name", "") or ""
            url = asset.get("browser_download_url", "") or ""
            if url and rx.search(name):
                return url
    return None

patterns = [
    rf"bossac[-_]?{re.escape(version)}.*x86_64.*linux.*\.(tar\.gz|tgz)$",
    rf"bossac.*x86_64.*linux.*\.(tar\.gz|tgz)$",
    rf"bossac.*linux.*x86_64.*\.(tar\.gz|tgz)$",
    rf"bossac.*(amd64|x86_64).*linux.*\.(tar\.gz|tgz)$",
    rf"bossac[-_]?{re.escape(version)}.*x86_64.*linux.*\.zip$",
    rf"bossac.*(amd64|x86_64).*linux.*\.zip$",
]

url = pick(patterns)
if not url:
    raise SystemExit("No suitable Linux x86_64 bossac asset found in BOSSA release assets")

print(url)
PY
  )"
fi

archive_path="${tmpdir}/bossac.archive"
curl -fsSL -o "${archive_path}" "${bossac_url}"

if [[ "$(head -c 2 "${archive_path}" || true)" == "PK" ]]; then
  if ! command -v unzip >/dev/null 2>&1; then
    echo "ERROR: unzip is required to extract bossac from ${bossac_url}" >&2
    exit 1
  fi
  unzip -q -o "${archive_path}" -d "${tmpdir}"
else
  if ! gzip -t "${archive_path}" >/dev/null 2>&1; then
    echo "ERROR: Downloaded bossac archive is not a valid gzip stream: ${bossac_url}" >&2
    echo "First bytes:" >&2
    head -c 256 "${archive_path}" >&2 || true
    echo >&2
    exit 1
  fi
  tar -xzf "${archive_path}" -C "${tmpdir}"
fi

bossac_src="$(
  find "${tmpdir}" -maxdepth 4 -type f -name bossac -perm -111 2>/dev/null | head -n 1 || true
)"
if [[ -z "${bossac_src}" ]]; then
  echo "ERROR: Unable to locate extracted 'bossac' binary from ${bossac_url}" >&2
  exit 1
fi

install -m755 "${bossac_src}" "${dest_dir}/bossac"

echo "Fetched bossac ${BOSSAC_VERSION} to ${dest_dir}/bossac"
