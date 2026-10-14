#!/usr/bin/env python3
"""Collect complete instruction data from the existing ISF inspector."""

import argparse
import json
import subprocess
from pathlib import Path


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--parser", type=Path, required=True)
    ap.add_argument("--decoded", type=Path, required=True)
    ap.add_argument("--output", type=Path, required=True)
    args = ap.parse_args()

    scripts = {}
    for src in sorted(args.decoded.glob("*.ISF")):
        name = src.stem
        result = subprocess.run(
            ["python3", str(args.parser), str(src), "--json"],
            check=True, text=True, capture_output=True,
        )
        script = json.loads(result.stdout)
        if not script["instructions"]:
            raise SystemExit(f"No instructions found in {src}")
        scripts[name] = script
        print(f"{name}: {len(script['instructions'])} instructions")

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(scripts, ensure_ascii=False), encoding="utf-8")
    print(f"wrote {args.output}")


if __name__ == "__main__":
    main()
