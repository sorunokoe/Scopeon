#!/usr/bin/env node
/**
 * postinstall.js — download the correct platform binary from GitHub Releases.
 *
 * Platform/arch → release asset mapping:
 *   darwin  arm64  → scopeon-aarch64-apple-darwin
 *   darwin  x64    → scopeon-x86_64-apple-darwin
 *   linux   x64    → scopeon-x86_64-unknown-linux-musl
 *   linux   arm64  → scopeon-aarch64-unknown-linux-musl
 *   win32   x64    → scopeon-x86_64-pc-windows-msvc.exe
 */

const https = require("https");
const fs = require("fs");
const path = require("path");
const { execSync } = require("child_process");
const { createGunzip } = require("zlib");
const { Extract } = require("tar");

const VERSION = require("./package.json").version;
const REPO = "scopeon/scopeon";
const BASE_URL = `https://github.com/${REPO}/releases/download/v${VERSION}`;

function assetName() {
  const p = process.platform;
  const a = process.arch;
  if (p === "darwin" && a === "arm64") return "scopeon-aarch64-apple-darwin.tar.gz";
  if (p === "darwin" && a === "x64")  return "scopeon-x86_64-apple-darwin.tar.gz";
  if (p === "linux"  && a === "x64")  return "scopeon-x86_64-unknown-linux-musl.tar.gz";
  if (p === "linux"  && a === "arm64") return "scopeon-aarch64-unknown-linux-musl.tar.gz";
  if (p === "win32"  && a === "x64")  return "scopeon-x86_64-pc-windows-msvc.zip";
  throw new Error(`Unsupported platform: ${p} ${a}. Build from source: https://github.com/${REPO}`);
}

const binDir = path.join(__dirname, "bin");
const binExt = process.platform === "win32" ? ".exe" : "";
const binPath = path.join(binDir, `scopeon${binExt}`);

if (!fs.existsSync(binDir)) fs.mkdirSync(binDir, { recursive: true });

const asset = assetName();
const url = `${BASE_URL}/${asset}`;

console.log(`[scopeon] Downloading ${url}`);

function download(url, cb) {
  https.get(url, (res) => {
    if (res.statusCode === 302 || res.statusCode === 301) {
      return download(res.headers.location, cb);
    }
    if (res.statusCode !== 200) {
      throw new Error(`Download failed: HTTP ${res.statusCode}`);
    }
    cb(res);
  }).on("error", (e) => { throw e; });
}

download(url, (stream) => {
  if (asset.endsWith(".tar.gz")) {
    const gunzip = createGunzip();
    const extract = new Extract({ cwd: binDir, strip: 0 });
    stream.pipe(gunzip).pipe(extract);
    extract.on("finish", () => {
      fs.chmodSync(binPath, 0o755);
      console.log("[scopeon] Installed at", binPath);
    });
  } else {
    // Windows .zip
    const tmp = path.join(binDir, "scopeon.zip");
    const out = fs.createWriteStream(tmp);
    stream.pipe(out);
    out.on("finish", () => {
      execSync(`powershell -command "Expand-Archive -Path '${tmp}' -DestinationPath '${binDir}' -Force"`);
      fs.unlinkSync(tmp);
      console.log("[scopeon] Installed at", binPath);
    });
  }
});
