#!/usr/bin/env python3
"""uv run --with pyte scripts/tui_visual_check.py；通过测试闸门采集当前渲染器。"""
import argparse
import errno
import fcntl
import html
import json
import os
from pathlib import Path
import pty
import select
import struct
import subprocess
import tempfile
import termios
import time

import pyte
from wcwidth import wcswidth

ROOT = Path(__file__).resolve().parent.parent
PALETTE = {"black": "#20252d", "white": "#c4c7e5", "red": "#ed777b", "green": "#6acf9d",
           "brown": "#e8c45c", "yellow": "#e8c45c", "blue": "#7c9de5", "magenta": "#c698db",
           "cyan": "#73c8d0", "brightblack": "#707784", "brightwhite": "#ffffff"}


def test_binary():
    result = subprocess.run(["cargo", "test", "--bin", "nix-pins", "--no-run", "--message-format=json"],
                            cwd=ROOT, check=True, text=True, stdout=subprocess.PIPE)
    for line in result.stdout.splitlines():
        if line.startswith("{"):
            row = json.loads(line)
            if row.get("reason") == "compiler-artifact" and row.get("profile", {}).get("test") and row.get("executable"):
                return Path(row["executable"])
    raise RuntimeError("test binary not found")


def screen_html(screen):
    def color(value, fallback):
        if value == "default":
            return fallback
        return PALETTE.get(value, f"#{value}" if len(value) == 6 else value)
    rows = []
    for y in range(screen.lines):
        cells = []
        x = 0
        while x < screen.columns:
            cell = screen.buffer[y][x]
            foreground = color(cell.fg, "#c4c7e5")
            background = color(cell.bg, "#20252d")
            if cell.reverse:
                foreground, background = background, foreground
            # 宽字符跨多个终端单元格，跳过其 continuation cell，防止回放重复占位。
            width = max(1, wcswidth(cell.data))
            style = f"color:{foreground};background:{background};font-weight:{600 if cell.bold else 400};width:{width * 11}px"
            cells.append(f'<span style="{style}">{html.escape(cell.data or " ")}</span>')
            x += width
        rows.append('<div class="row">' + ''.join(cells) + '</div>')
    return ('<!doctype html><meta charset="utf-8"><title>PTY frame replay</title><style>'
            'body{margin:16px;background:#20252d}.row{height:24px;white-space:nowrap}'
            '.row span{display:inline-block;width:11px;height:24px;line-height:24px;'
            'vertical-align:top;white-space:pre;font:18px/24px Menlo,monospace}</style>' + ''.join(rows))


def screen_text(screen):
    # pyte 在中文被动态覆盖后可能留下空 continuation cell，直接读缓冲区避免 display 崩溃。
    return '\n'.join(''.join(screen.buffer[y][x].data for x in range(screen.columns))
                     for y in range(screen.lines))


def capture(binary, scene, size, animation_frames=None):
    with tempfile.TemporaryDirectory(prefix="nix-pins-visual-") as temporary:
        gate = Path(temporary)
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", size[1], size[0], 0, 0))
        environment = dict(os.environ, TERM="xterm-256color", TUI_SCENE=scene, TUI_GATE=str(gate))
        environment.pop("NO_COLOR", None)
        def terminal():
            os.setsid()
            fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
        command = [str(binary), "progress::visual_tests::capture_state", "--exact", "--ignored", "--nocapture", "--test-threads=1"]
        process = subprocess.Popen(["sh", "-c", 'stty -g > before; "$@"; code=$?; stty -g > after; exit "$code"',
                                    "visual-check", *command], cwd=gate, env=environment,
                                   stdin=slave, stdout=slave, stderr=slave, preexec_fn=terminal)
        os.close(slave)
        output = bytearray()
        ready = None
        started = time.monotonic()
        snapshot = None
        last_read = started
        next_frame = 0.2
        try:
            while time.monotonic() - started < 15:
                if select.select([master], [], [], 0.03)[0]:
                    try:
                        chunk = os.read(master, 65536)
                    except OSError as error:
                        if error.errno == errno.EIO:
                            break
                        raise
                    if not chunk:
                        break
                    output.extend(chunk)
                    last_read = time.monotonic()
                # ready/release 与测试进程握手，采集固定状态时仍保留真实渲染动画。
                if ready is None and (gate / "ready").exists():
                    ready = time.monotonic()
                if ready is not None and snapshot is None and time.monotonic() - last_read > 0.02:
                    elapsed = time.monotonic() - ready
                    if animation_frames is not None and elapsed >= next_frame:
                        animation_frames.append((elapsed, bytes(output)))
                        next_frame = elapsed + 0.2
                    # 动画验证多采几帧；release 只在最后一帧确定后发送。
                    if elapsed > (1.6 if animation_frames is not None else 0.5):
                        snapshot = bytes(output)
                        (gate / "release").touch()
                if process.poll() is not None:
                    break
            process.wait(timeout=3)
            assert process.returncode == 0, bytes(output)[-2000:]
            assert snapshot is not None, f"scene was not reached: {scene}"
            assert (gate / "before").read_bytes() == (gate / "after").read_bytes(), "terminal mode was not restored"
            assert output.count(b"\x1b[?1049h") == output.count(b"\x1b[?1049l") == 1
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
            os.close(master)
        screen = pyte.Screen(*size)
        pyte.Stream(screen).feed(snapshot.split(b"\x1b[?1049h", 1)[1].decode("utf-8"))
        return screen, snapshot


def verify(scene, screen):
    text = screen_text(screen)
    assert not any('\u4e00' <= character <= '\u9fff' for character in text), 'TUI text must be in English'
    assert "⏸" not in text, "waiting must not use a paused icon"
    if scene in ("S02", "checking-32"):
        assert "Waiting to check" in text
    if scene == "S03":
        assert "Waiting to download" in text and "Waiting to check" not in text
    if scene == "S08":
        assert "Waiting to build" in text and "Source ready" not in text
    if scene == "S10":
        assert "frontend" in text and "Waiting to build" in text
    if screen.columns >= 100:
        assert "Information" in text, scene
    if scene in ("S02", "S03", "S21", "checking-32"):
        assert "forge 7ae13f0" in text, text
        assert "7ae13f0" + "1" * 33 not in text or scene == "S21"
    if scene == "S21":
        assert "1234567aaaa → 1234567bbbb" in text
        assert "hex-tag 7ae13f0" + "1" * 33 in text
    if scene == "S01":
        assert "Loading configuration" in text and "bat" not in text
    if scene == "S04":
        assert "60%" in text and "50%" in text
    if scene == "S05":
        assert "objects" in text and "64%" in text
    if scene == "S06":
        assert "%" not in text and "12.0" in text
    if scene == "S14":
        assert "frontend" in text and "npmDepsHash" in text and "server" not in text
    if scene in ("S02", "checking-32", "S06"):
        assert '▄' not in text, "unknown totals must not display a moving block"
        if scene in ("S02", "checking-32"):
            row = next(line for line in text.splitlines() if "bat v0.24.0" in line)
            identity, stage = row.lstrip('│').split('│', 1)
            assert "Checking" not in identity and "Checking" in stage, row


def main():
    parser = argparse.ArgumentParser(description="Capture the current TUI renderer through the test gate")
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--output", type=Path, default=ROOT / "target/tui-validation/latest/renderer")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    temporary_pages = tempfile.TemporaryDirectory(prefix="nix-pins-renderer-pages-")
    pages = Path(temporary_pages.name)
    binary = (args.binary or test_binary()).resolve()
    listing = subprocess.check_output(
        ["cargo", "run", "--quiet", "--example", "tui", "--", "--list"], cwd=ROOT, text=True
    )
    scenes = [line.split(maxsplit=1)[0] for line in listing.splitlines()]
    captures = [(scene, (160, 34)) for scene in scenes]
    captures += [("S02", (80, 24)), ("S04", (80, 24)), ("S14", (80, 24))]
    session = f"nix-pins-tui-visual-{os.getpid()}"
    def browser(*command):
        subprocess.run(["agent-browser", "--session", session, *map(str, command)], check=True,
                       stdout=subprocess.DEVNULL)
    results = []
    failures = []
    try:
        for scene, size in captures:
            screen, ansi = capture(binary, scene, size)
            name = f"{scene}-{size[0]}x{size[1]}"
            (args.output / f"{name}.ansi").write_bytes(ansi)
            (args.output / f"{name}.txt").write_text(screen_text(screen))
            error = None
            try:
                verify(scene, screen)
            except AssertionError as failure:
                error = str(failure)
                failures.append(name)
            page = pages / f"{name}.html"
            page.write_text(screen_html(screen))
            browser("open", page.as_uri())
            browser("set", "viewport", size[0] * 11 + 32, size[1] * 24 + 32)
            browser("screenshot", args.output / f"{name}.png")
            results.append({"scene": scene, "size": size, "image": f"{name}.png", "checks": "failed" if error is not None else "passed", "error": error})
            print(f"{'FAIL' if error is not None else 'PASS'} {name}", flush=True)
    finally:
        browser("close")
        temporary_pages.cleanup()
    (args.output / "renderer-results.json").write_text(json.dumps(results, ensure_ascii=False, indent=2))
    if failures:
        raise SystemExit("failed frames saved: " + ', '.join(failures))


if __name__ == "__main__":
    main()
