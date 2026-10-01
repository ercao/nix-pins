"""直接校验文档中的完整 Pin 声明；--live 执行快速开始并用 Reader 构建源码。"""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile


def run(command, **kwargs):
    result = subprocess.run(command, text=True, capture_output=True, **kwargs)
    if result.returncode:
        raise SystemExit(result.stdout + result.stderr)
    return result.stdout


def blocks(page):
    return re.findall(r"^```nix\n(.*?)^```", page.read_text(), re.MULTILINE | re.DOTALL)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--live", action="store_true")
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[2]
    content = repo / "docs/content"
    evaluator = repo / "nix/evaluator.nix"
    count = 0
    with tempfile.TemporaryDirectory(prefix="nix-pins-docs-") as directory:
        directory = Path(directory)
        config = directory / "pins-config.nix"
        for page in sorted(content.rglob("*.md")):
            for block in blocks(page):
                if not re.match(r"\{\s*pin\b[^}]*\}\s*:", block):
                    continue
                config.write_text(block)
                expression = f"let pkgs = import <nixpkgs> {{}}; pins = import {evaluator} {{ inherit pkgs; config = {config}; }}; in builtins.mapAttrs (_: pin: pin.check) pins"
                try:
                    run(["nix", "eval", "--impure", "--json", "--expr", expression])
                except SystemExit as error:
                    raise SystemExit(f"{page.relative_to(content)}:\n{error}") from error
                count += 1
        if not count:
            raise SystemExit("未找到可校验的完整 Pin 声明")
        print(f"通过：{count} 个完整 Pin 声明通过当前公开 Nix API 校验")
        if args.live:
            tutorial = content / "zh-CN/1.getting-started/2.first-pin.md"
            config.write_text(blocks(tutorial)[0])
            executable = os.environ.get("NIX_PINS_BIN")
            command = [executable] if executable else ["cargo", "run", "--quiet", "--manifest-path", str(repo / "Cargo.toml"), "--"]
            pins_file = directory / "pins.json"
            run(command + ["update", "--config", str(config), "--pins", str(pins_file)], cwd=directory)
            data = json.loads(pins_file.read_text())
            pin = data["pins"]["patchelf"]
            assert data["schemaVersion"] == 2 and pin["version"]
            assert pin["sources"]["default"]["hash"].startswith("sha256-")
            run(command + ["status", "--config", str(config), "--pins", str(pins_file)], cwd=directory)
            expression = f"let pins = import {repo / 'nix/pins.nix'} {{ pkgs = import <nixpkgs> {{}}; file = {pins_file}; }}; in pins.patchelf.sources.default.src"
            run(["nix", "build", "--impure", "--no-link", "--expr", expression])
            print(f"通过：真实快速开始，Version={pin['version']}，源码哈希与 Reader 构建有效")


if __name__ == "__main__":
    main()
