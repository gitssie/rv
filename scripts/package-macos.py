#!/usr/bin/env python3
"""Package RV as a signed or ad-hoc signed macOS app."""

import argparse
from pathlib import Path
import plistlib
import shutil
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--version", default="0.2.0")
    parser.add_argument("--identity", help="Developer ID Application signing identity")
    args = parser.parse_args()
    if not args.binary.is_file():
        parser.error("--binary must name an existing executable")
    if args.output.suffix != ".app" or args.output.exists():
        parser.error("--output must name a new .app bundle")

    bundle_id = "io.github.madeye.rv"
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="rv-package-", dir=args.output.parent) as work:
        app = Path(work) / "RV.app"
        contents = app / "Contents"
        (contents / "MacOS").mkdir(parents=True)
        (contents / "Resources").mkdir()
        shutil.copy2(args.binary, contents / "MacOS/rv")
        assets = Path(__file__).resolve().parent.parent / "assets"
        shutil.copy2(assets / "app-icon.icns", contents / "Resources/app-icon.icns")
        info = {
            "CFBundleName": "RV", "CFBundleDisplayName": "RV",
            "CFBundleIdentifier": bundle_id, "CFBundleExecutable": "rv",
            "CFBundleIconFile": "app-icon.icns", "CFBundlePackageType": "APPL",
            "CFBundleShortVersionString": args.version, "CFBundleVersion": args.version,
            "LSMinimumSystemVersion": "12.0", "NSHighResolutionCapable": True,
            "NSSupportsAutomaticGraphicsSwitching": True,
        }
        with (contents / "Info.plist").open("wb") as file:
            plistlib.dump(info, file)
        command = ["codesign", "--force", "--sign", args.identity or "-"]
        if args.identity:
            command += ["--options", "runtime", "--timestamp"]
        subprocess.run(command + [str(app)], check=True)
        subprocess.run(["codesign", "--verify", "--deep", "--strict", str(app)], check=True)
        shutil.move(str(app), args.output)
    print(f"Packaged {args.output}")


if __name__ == "__main__":
    main()
