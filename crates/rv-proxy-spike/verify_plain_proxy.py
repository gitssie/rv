#!/usr/bin/env python3
"""Verify a patched meow checkout without modifying RV manifests or lockfile."""
import argparse
import json
from pathlib import Path
import re
import shutil
import subprocess
import tempfile

BASE = "59da792d91f10598ce1d9f0afcb99be59150c22c"
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("meow_checkout", type=Path)
args = parser.parse_args()
meow = args.meow_checkout.resolve()
crate = Path(__file__).resolve().parent
rv = crate.parents[1]
subprocess.run(["git", "merge-base", "--is-ancestor", BASE, "HEAD"], cwd=meow, check=True)
patch_file = crate / "patches/optional-proxy-tls.patch"
actual = subprocess.check_output(["git", "diff", BASE, "--", "crates/meow-proxy"], cwd=meow, text=True)
if actual.replace("\n \n", "\n\n") != patch_file.read_text():
    parser.error("checkout must match the supplied patch against the pinned upstream revision")
subprocess.run(["git", "apply", "--reverse", "--check", str(crate / "patches/optional-proxy-tls.patch")], cwd=meow, check=True)
config = []
for name in ("meow-proxy", "meow-common"):
    config += ["--config", 'patch."https://github.com/meow-rs/meow-rs".' + name + ".path=" + json.dumps(str(meow / "crates" / name))]
with tempfile.TemporaryDirectory(prefix="rv-plain-proxy-") as directory:
    workspace = Path(directory)
    manifest = (rv / "Cargo.toml").read_text()
    manifest = re.sub(r"^members = .*", 'members = ["crates/rv-proxy-spike"]', manifest, count=1, flags=re.M)
    manifest = re.sub(r"^default-members = .*", 'default-members = ["crates/rv-proxy-spike"]', manifest, count=1, flags=re.M)
    (workspace / "Cargo.toml").write_text(manifest)
    shutil.copyfile(rv / "Cargo.lock", workspace / "Cargo.lock")
    shutil.copytree(crate, workspace / "crates/rv-proxy-spike")
    command = ["cargo", *config]
    target = ["--target-dir", str(rv / "target")]
    tree = subprocess.check_output(command + ["tree", "-p", "rv-proxy-spike", "--features", "openssl-compat", "--prefix", "none"], cwd=workspace, text=True)
    if re.search(r"^(boring|boring-sys|tokio-boring) v", tree, flags=re.M):
        raise SystemExit("unexpected BoringSSL dependency")
    subprocess.run(command + ["test", "-p", "rv-proxy-spike", "--features", "openssl-compat", *target], cwd=workspace, check=True)
    print("PASS: compatibility tests passed; no BoringSSL in the selected dependency tree")
