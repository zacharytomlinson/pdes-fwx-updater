#!/usr/bin/env bash

set -euo pipefail

BOSSAC_VERSION="${BOSSAC_VERSION:-1.9.1}"
BOSSAC_URL="${BOSSAC_URL:-https://github.com/shumatech/BOSSA/releases/download/${BOSSAC_VERSION}/bossac-${BOSSAC_VERSION}-x86_64-linux.tar.gz}"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
dest_dir="${repo_root}/tools/linux"

mkdir -p "${dest_dir}"

tmpdir="$(mktemp -d)"
trap 'rm -rf "${tmpdir}"' EXIT

curl -L -o "${tmpdir}/bossac.tar.gz" "${BOSSAC_URL}"
tar -xzf "${tmpdir}/bossac.tar.gz" -C "${tmpdir}"
install -m755 "${tmpdir}/bossac" "${dest_dir}/bossac"

echo "Fetched bossac ${BOSSAC_VERSION} to ${dest_dir}/bossac"

