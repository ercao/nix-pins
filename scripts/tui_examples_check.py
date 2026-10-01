#!/usr/bin/env python3
"""uv run --with pyte scripts/tui_examples_check.py；验证可运行示例并保存全部场景截图。"""
import errno
import argparse
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
from tui_visual_check import screen_html, screen_text, verify

ROOT = Path(__file__).resolve().parent.parent
BINARY = ROOT / 'target/debug/examples/tui'


def capture(scene, size, key=b'q'):
    with tempfile.TemporaryDirectory(prefix='nix-pins-example-') as temporary:
        folder = Path(temporary)
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', size[1], size[0], 0, 0))
        environment = dict(os.environ, TERM='xterm-256color')
        environment.pop('NO_COLOR', None)
        def terminal():
            os.setsid()
            fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
        process = subprocess.Popen(['sh', '-c', 'stty -g > before; "$@"; code=$?; stty -g > after; exit "$code"',
                                    'tui-example', str(BINARY), '--scene', scene], cwd=folder, env=environment,
                                   stdin=slave, stdout=slave, stderr=slave, preexec_fn=terminal)
        os.close(slave)
        raw = bytearray()
        snapshot = None
        started = last_read = time.monotonic()
        try:
            while time.monotonic() - started < 8:
                if select.select([master], [], [], 0.03)[0]:
                    try:
                        chunk = os.read(master, 65536)
                    except OSError as error:
                        if error.errno == errno.EIO:
                            break
                        raise
                    if not chunk:
                        break
                    raw.extend(chunk)
                    last_read = time.monotonic()
                # 在两次终端写入之间保存帧，再发送退出键验证模式恢复。
                if snapshot is None and time.monotonic()-started > 0.6 and time.monotonic()-last_read > 0.02:
                    snapshot = bytes(raw)
                    os.write(master, key)
                if process.poll() is not None:
                    break
            process.wait(timeout=3)
            assert process.returncode == 0 and snapshot is not None, bytes(raw)[-2000:]
            assert (folder/'before').read_bytes() == (folder/'after').read_bytes(), 'terminal mode was not restored'
            assert raw.count(b'\x1b[?1049h') == raw.count(b'\x1b[?1049l') == 1
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
            os.close(master)
        screen = pyte.Screen(*size)
        pyte.Stream(screen).feed(snapshot.split(b'\x1b[?1049h', 1)[1].decode('utf-8'))
        return screen, snapshot


def main():
    parser = argparse.ArgumentParser(description="Capture all TUI example states and layouts in a PTY")
    parser.add_argument('--output', type=Path, default=ROOT / 'target/tui-validation/latest/examples')
    args = parser.parse_args()
    output = args.output.resolve()
    subprocess.run(['cargo', 'build', '--example', 'tui'], cwd=ROOT, check=True)
    listing = subprocess.check_output([BINARY, '--list'], text=True)
    scenes = [line.split(maxsplit=1) for line in listing.splitlines()]
    output.mkdir(parents=True, exist_ok=True)
    temporary_pages = tempfile.TemporaryDirectory(prefix='nix-pins-tui-pages-')
    pages = Path(temporary_pages.name)
    session = f'nix-pins-tui-examples-{os.getpid()}'
    def browser(*args):
        subprocess.run(['agent-browser', '--session', session, *map(str, args)], check=True, stdout=subprocess.DEVNULL)
    records = []
    cases = [(scene, title, (160, 34)) for scene, title in scenes]
    cases += [('all', 'All states - narrow terminal', (80, 24)), ('all', 'All states - minimum layout', (60, 12))]
    try:
        for scene, title, size in cases:
            screen, raw = capture(scene, size)
            text = screen_text(screen)
            if scene not in ('all', 'success', 'status-colors'):
                verify(scene, screen)
            if scene == 'all' and size[0] == 160:
                assert not any('\u4e00' <= character <= '\u9fff' for character in text), 'TUI text must be in English'
                for expected in ['Waiting to check', 'Waiting to download', 'Waiting to build', '60%', '64%', 'npmDepsHash',
                                 'Applying patches', 'Resolving sources', 'Resolving derived hashes',
                                 'Source reused', 'nested v1 → v2', 'Done', 'Messages', 'INFO', 'DONE', 'FAIL']:
                    assert expected in text, (expected, text)
                assert all(icon in text for icon in ['○', '✔', '⚠'])
            name = f'{scene}-{size[0]}x{size[1]}'
            (output/(name+'.ansi')).write_bytes(raw)
            (output/(name+'.txt')).write_text(text)
            page = pages/(name+'.html')
            page.write_text(screen_html(screen))
            browser('open', page.as_uri())
            browser('set', 'viewport', size[0]*11+32, size[1]*24+32)
            browser('screenshot', output/(name+'.png'))
            records.append({'scene': scene, 'title': title, 'size': size, 'image': name+'.png',
                            'checks': 'passed', 'terminal_restored': True})
            print('PASS '+name, flush=True)
        for key in [b'\x1b', b'\x03']:
            capture('all', (160, 34), key)
    finally:
        browser('close')
        temporary_pages.cleanup()
        (output/'results.json').write_text(json.dumps(records, ensure_ascii=False, indent=2))
    sections = ''.join(f'<section><h2>{html.escape(row["title"])} · {row["size"][0]} × {row["size"][1]}</h2><code>cargo run --example tui -- --scene {row["scene"]}</code><a href="{row["image"]}"><img loading="lazy" src="{row["image"]}"></a></section>' for row in records)
    (output/'index.html').write_text('''<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>TUI states and layouts</title><style>body{margin:24px;background:#f4f5f7;color:#20252d;font:16px/1.6 system-ui}main{max-width:1800px;margin:auto}img{width:100%;display:block}section{margin:28px 0;border-top:1px solid #cdd2d9}code{display:block;margin:10px 0}a{color:#3658a7}</style><main><h1>TUI states and layouts</h1><p>Run <code>cargo run --example tui</code> to preview all states. The example uses fixed data and animated active icons. Use j/k to scroll tasks, [/] to scroll messages, and q/Esc/Ctrl+C to quit. It does not download, evaluate Nix, build, or write the pins file.</p>'''+f'<p>{len(scenes)} scenes and {len(records)} PTY frame screenshots. Every scene restores the terminal on exit. Wide terminals show Information; narrow layouts allow scrolling through all tasks.</p>'+'<p><a href="results.json">Check results</a></p>'+sections+'</main></html>')
    print(f'PASS {len(records)} screenshots; q/Esc/Ctrl+C restore terminal')


if __name__ == '__main__':
    main()
