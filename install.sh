#!/usr/bin/env sh
# Scopeon installer — downloads the correct pre-built binary for your platform.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/scopeon/scopeon/main/install.sh | sh
#
# Options (via environment variables):
#   SCOPEON_VERSION  — version to install (default: latest)
#   SCOPEON_INSTALL  — installation directory (default: ~/.local/bin)
#
# Requirements: curl, tar (or unzip on Windows via WSL)

set -eu

# ── Config ────────────────────────────────────────────────────────────────────
REPO="scopeon/scopeon"
INSTALL_DIR="${SCOPEON_INSTALL:-$HOME/.local/bin}"
BINARY="scopeon"

# ── Detect platform ───────────────────────────────────────────────────────────
detect_target() {
    local os arch
    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Darwin)
            case "$arch" in
                arm64)  echo "aarch64-apple-darwin" ;;
                x86_64) echo "x86_64-apple-darwin" ;;
                *)      die "Unsupported macOS architecture: $arch" ;;
            esac
            ;;
        Linux)
            case "$arch" in
                x86_64)  echo "x86_64-unknown-linux-gnu" ;;
                aarch64) echo "aarch64-unknown-linux-gnu" ;;
                arm64)   echo "aarch64-unknown-linux-gnu" ;;
                *)       die "Unsupported Linux architecture: $arch" ;;
            esac
            ;;
        MINGW*|MSYS*|CYGWIN*)
            echo "x86_64-pc-windows-msvc"
            ;;
        *)
            die "Unsupported operating system: $os"
            ;;
    esac
}

# ── Utilities ─────────────────────────────────────────────────────────────────
die() {
    printf "error: %s\n" "$1" >&2
    printf "Falling back to: cargo install scopeon\n" >&2
    exit 1
}

info() { printf "  \033[32m✓\033[0m %s\n" "$1"; }
warn() { printf "  \033[33m!\033[0m %s\n" "$1"; }

# ── Resolve version ───────────────────────────────────────────────────────────
resolve_version() {
    if [ -n "${SCOPEON_VERSION:-}" ]; then
        echo "$SCOPEON_VERSION"
        return
    fi
    # Query GitHub API for latest release tag
    local version
    version="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
        | grep '"tag_name"' \
        | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/')"
    if [ -z "$version" ]; then
        die "Could not determine latest version. Set SCOPEON_VERSION manually."
    fi
    echo "$version"
}

# ── Main ──────────────────────────────────────────────────────────────────────
main() {
    printf "\n\033[1m🔬 Installing Scopeon — AI context observability\033[0m\n\n"

    local target version archive_name url tmp_dir

    target="$(detect_target)"
    version="$(resolve_version)"
    info "Version: $version"
    info "Target:  $target"

    # Build archive filename
    case "$target" in
        *windows*) archive_name="scopeon-${version}-${target}.zip" ;;
        *)         archive_name="scopeon-${version}-${target}.tar.gz" ;;
    esac

    url="https://github.com/$REPO/releases/download/$version/$archive_name"
    info "Downloading $archive_name..."

    # Download to a temp directory
    tmp_dir="$(mktemp -d)"
    trap 'rm -rf "$tmp_dir"' EXIT

    if ! curl -fsSL --progress-bar "$url" -o "$tmp_dir/$archive_name"; then
        die "Download failed: $url"
    fi

    # Extract
    info "Extracting..."
    case "$archive_name" in
        *.tar.gz)
            tar xzf "$tmp_dir/$archive_name" -C "$tmp_dir" "$BINARY" 2>/dev/null || \
            tar xzf "$tmp_dir/$archive_name" -C "$tmp_dir"
            ;;
        *.zip)
            unzip -q "$tmp_dir/$archive_name" -d "$tmp_dir"
            ;;
    esac

    # Install
    mkdir -p "$INSTALL_DIR"
    install -m 755 "$tmp_dir/$BINARY" "$INSTALL_DIR/$BINARY"

    info "Installed to $INSTALL_DIR/$BINARY"

    # PATH hint
    case ":$PATH:" in
        *":$INSTALL_DIR:"*) ;;
        *)
            warn "Add $INSTALL_DIR to your PATH:"
            printf "\n    export PATH=\"%s:\$PATH\"\n\n" "$INSTALL_DIR"
            ;;
    esac

    printf "\n\033[1m✅ Scopeon $version installed!\033[0m\n"
    printf "\nQuick start:\n"
    printf "  scopeon init    # configure Claude Code integration\n"
    printf "  scopeon         # start the TUI dashboard\n\n"
}

main "$@"
