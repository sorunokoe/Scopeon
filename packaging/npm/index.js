#!/usr/bin/env node
// index.js — tiny shim that invokes the downloaded platform binary.
const { spawnSync } = require("child_process");
const path = require("path");
const ext = process.platform === "win32" ? ".exe" : "";
const bin = path.join(__dirname, "bin", `scopeon${ext}`);
const result = spawnSync(bin, process.argv.slice(2), { stdio: "inherit" });
process.exit(result.status ?? 1);
