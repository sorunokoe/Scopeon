"""scopeon — AI context observability for Claude Code, Codex, Cursor, and every LLM coding tool.

This package is a thin Python wrapper that downloads and runs the native scopeon
binary for your platform. The binary is downloaded on first run and cached in
~/.cache/scopeon/bin/.
"""

from __future__ import annotations

import os
import platform
import stat
import subprocess
import sys
import tarfile
import tempfile
import urllib.request
from pathlib import Path

VERSION = "0.6.0"
REPO = "scopeon/scopeon"
CACHE_DIR = Path.home() / ".cache" / "scopeon" / "bin"


def _asset_name() -> str:
    s = platform.system().lower()
    m = platform.machine().lower()
    if s == "darwin" and m in ("arm64", "aarch64"):
        return "scopeon-aarch64-apple-darwin.tar.gz"
    if s == "darwin" and m == "x86_64":
        return "scopeon-x86_64-apple-darwin.tar.gz"
    if s == "linux" and m == "x86_64":
        return "scopeon-x86_64-unknown-linux-musl.tar.gz"
    if s == "linux" and m in ("aarch64", "arm64"):
        return "scopeon-aarch64-unknown-linux-musl.tar.gz"
    if s == "windows" and m == "amd64":
        return "scopeon-x86_64-pc-windows-msvc.zip"
    raise SystemExit(
        f"scopeon: unsupported platform {s}/{m}. "
        f"Build from source: https://github.com/{REPO}"
    )


def _bin_path() -> Path:
    ext = ".exe" if platform.system().lower() == "windows" else ""
    return CACHE_DIR / f"scopeon{ext}"


def _ensure_binary() -> Path:
    bp = _bin_path()
    if bp.exists():
        return bp
    CACHE_DIR.mkdir(parents=True, exist_ok=True)
    asset = _asset_name()
    url = f"https://github.com/{REPO}/releases/download/v{VERSION}/{asset}"
    print(f"[scopeon] Downloading {url}", file=sys.stderr)
    with tempfile.NamedTemporaryFile(delete=False, suffix=asset) as tmp:
        urllib.request.urlretrieve(url, tmp.name)
        if asset.endswith(".tar.gz"):
            with tarfile.open(tmp.name, "r:gz") as tf:
                for member in tf.getmembers():
                    if member.name.endswith("scopeon") or member.name.endswith("scopeon.exe"):
                        member.name = os.path.basename(member.name)
                        tf.extract(member, CACHE_DIR)
                        break
        else:
            import zipfile
            with zipfile.ZipFile(tmp.name) as zf:
                for name in zf.namelist():
                    if name.endswith("scopeon.exe"):
                        zf.extract(name, CACHE_DIR)
                        break
        os.unlink(tmp.name)
    bp.chmod(bp.stat().st_mode | stat.S_IEXEC | stat.S_IXGRP | stat.S_IXOTH)
    return bp


def main() -> None:
    bp = _ensure_binary()
    result = subprocess.run([str(bp)] + sys.argv[1:])
    sys.exit(result.returncode)


if __name__ == "__main__":
    main()
