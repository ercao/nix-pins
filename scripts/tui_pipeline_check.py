#!/usr/bin/env python3
"""uv run --with pyte scripts/tui_pipeline_check.py；采集完整 CLI 的流水线画面。"""
import argparse
import errno
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import struct
import subprocess
import sys
import tempfile
import termios
import time

import pyte
from tui_visual_check import screen_html, screen_text

ROOT = Path(__file__).resolve().parent.parent
# 受控 Nix 后端用文件闸门延迟慢 Source，验证跨 Pin 调度而不依赖网络时序。
FIXTURE = r'''
import json, sys, time
from pathlib import Path
def event(value):
    print('@nix ' + json.dumps(value), file=sys.stderr, flush=True)
if sys.argv[1] == 'eval':
    if 'p.check' in ' '.join(sys.argv):
        print(json.dumps({name: {'cmd': 'printf v2'} for name in ['alpha-fast', 'zeta-slow']}))
    else:
        def source(name, derived={}):
            return {'src': '/nix/store/' + name + '.drv', 'fetcher': {'url': {'url': 'https://example.invalid/' + name}}, 'derived': derived}
        print(json.dumps({
            'alpha-fast': {'sources': {'default': source('fast-source', {'vendorHash': '/nix/store/fast-derived.drv'})}},
            'zeta-slow': {'sources': {'default': source('slow-source')}}
        }))
else:
    name = sys.argv[-1]
    if 'slow-source' in name:
        Path('slow-started').touch()
        event({'action': 'start', 'id': 1, 'type': 101, 'fields': ['https://example.invalid/slow']})
        event({'action': 'result', 'id': 1, 'type': 105, 'fields': [12000000, 0, 0, 0]})
        while not Path('release').exists():
            time.sleep(0.01)
        Path('slow-completed').touch()
    elif 'fast-source' in name:
        Path('fast-source-completed').touch()
    elif 'fast-derived' in name:
        Path('fast-derived-completed').touch()
    print('error: hash mismatch\n  specified: sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=\n  got: sha256-pipeline', file=sys.stderr)
    sys.exit(1)
'''


def replay(raw, size, final=False):
    # 运行画面取 alternate screen，最终摘要取退出 alternate screen 之后的输出。
    screen = pyte.Screen(*size)
    frame = raw.rsplit(b'\x1b[?1049l', 1)[-1] if final else raw.split(b'\x1b[?1049h', 1)[1]
    pyte.Stream(screen).feed(frame.decode('utf-8'))
    return screen


def capture(binary, key=None):
    size = (160, 34)
    with tempfile.TemporaryDirectory(prefix='nix-pins-pipeline-') as temporary:
        folder = Path(temporary)
        fixture = folder / 'nix'
        fixture.write_text(f'#!{sys.executable}\n' + FIXTURE)
        fixture.chmod(0o755)
        (folder / 'pins-config.nix').write_text('{ pin }: {}\n')
        original = b'{"schemaVersion":2,"pins":{}}\n'
        (folder / 'pins.json').write_bytes(original)
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', size[1], size[0], 0, 0))
        environment = dict(os.environ, TERM='xterm-256color', PATH=f'{folder}:{os.environ["PATH"]}',
                           NIX_PINS_CHECKER_JOBS='2', NIX_PINS_DOWNLOAD_JOBS='2', NIX_PINS_HASH_JOBS='1')
        environment.pop('NO_COLOR', None)
        def terminal():
            os.setsid()
            fcntl.ioctl(slave, termios.TIOCSCTTY, 0)
        process = subprocess.Popen(['sh', '-c', 'stty -g > mode-before; "$@"; result=$?; stty -g > mode-after; exit "$result"',
                                    'pipeline-check', str(binary), 'update'], cwd=folder, env=environment,
                                   stdin=slave, stdout=slave, stderr=slave, preexec_fn=terminal)
        os.close(slave)
        raw = bytearray()
        snapshot = None
        ready_at = None
        started = time.monotonic()
        last_read = started
        try:
            while time.monotonic() - started < 15:
                if select.select([master], [], [], 0.03)[0]:
                    try:
                        chunk = os.read(master, 65536)
                    except OSError as error:
                        if error.errno == errno.EIO:
                            # Unix PTY 的从端关闭可表现为 EIO，与读到 EOF 一样结束采集。
                            break
                        raise
                    if not chunk:
                        break
                    raw.extend(chunk)
                    last_read = time.monotonic()
                slow = (folder / 'slow-started').exists()
                fast = (folder / 'fast-derived-completed').exists()
                if slow and fast and ready_at is None:
                    ready_at = time.monotonic()
                # 快任务完成后给渲染器留出刷新时间，再释放慢任务或触发取消。
                if snapshot is None and ready_at is not None and time.monotonic() - ready_at > 0.4 and time.monotonic() - last_read > 0.02:
                    screen = replay(bytes(raw), size)
                    text = screen_text(screen)
                    assert 'zeta-slow' in text and not (folder / 'slow-completed').exists()
                    assert '✔ alpha-fast' in text, text
                    assert 'Processing pins' in text, text
                    assert (folder / 'pins.json').read_bytes() == original
                    snapshot = bytes(raw)
                    if key:
                        os.write(master, key)
                    else:
                        (folder / 'release').touch()
                if process.poll() is not None:
                    break
            process.wait(timeout=3)
            assert snapshot is not None, bytes(raw)[-2000:]
            assert process.returncode == (130 if key else 0), bytes(raw)[-2000:]
            assert (folder / 'mode-before').read_bytes() == (folder / 'mode-after').read_bytes()
            assert raw.count(b'\x1b[?1049h') == raw.count(b'\x1b[?1049l') == 1
            if key:
                assert (folder / 'pins.json').read_bytes() == original
                assert b'Processed' not in raw
            else:
                pins = json.loads((folder / 'pins.json').read_bytes())['pins']
                assert pins['alpha-fast']['sources']['default']['derived']['vendorHash'] == 'sha256-pipeline'
                assert pins['zeta-slow']['sources']['default']['hash'] == 'sha256-pipeline'
            return size, snapshot, bytes(raw)
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
            os.close(master)


def main():
    parser = argparse.ArgumentParser(description="Capture and verify the independent pin pipeline in a PTY")
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/debug/nix-pins')
    parser.add_argument('--output', type=Path, default=ROOT / 'target/tui-validation/latest/pipeline')
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    temporary_pages = tempfile.TemporaryDirectory(prefix='nix-pins-pipeline-pages-')
    pages = Path(temporary_pages.name)
    session = f'nix-pins-pipeline-proof-{os.getpid()}'
    def browser(*command):
        subprocess.run(['agent-browser', '--session', session, *map(str, command)], check=True, stdout=subprocess.DEVNULL)
    records = []
    cases = [(None, None), (b'q', 'q'), (b'\x1b', 'esc'), (b'\x03', 'ctrl-c')]
    try:
        for key, label in cases:
            size, snapshot, raw = capture(args.binary.resolve(), key)
            frames = [('running', snapshot, False), ('complete', raw, True)] if not key else [(label, raw, True)]
            for state, frame, final in frames:
                name = state
                screen = replay(frame, size, final)
                (args.output / f'{name}.ansi').write_bytes(frame)
                (args.output / f'{name}.txt').write_text(screen_text(screen))
                page = pages / f'pipeline-{name}.html'
                page.write_text(screen_html(screen))
                browser('open', page.as_uri())
                browser('set', 'viewport', size[0] * 11 + 32, size[1] * 24 + 32)
                browser('screenshot', args.output / f'{name}.png')
                records.append({'state': state, 'image': f'{name}.png', 'checks': 'passed',
                                'exit': 130 if key else 0, 'terminal_restored': True, 'pins_unchanged_until_release': True})
            print(f'PASS {label or "pipeline"}', flush=True)
    finally:
        browser('close')
        temporary_pages.cleanup()
    (args.output / 'results.json').write_text(json.dumps(records, ensure_ascii=False, indent=2))
    pictures = ''.join(f'<figure><figcaption>{row["state"]}</figcaption><a href="{row["image"]}"><img src="{row["image"]}" alt="{row["state"]}"></a></figure>' for row in records)
    (args.output / 'index.html').write_text('''<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Independent pin pipeline</title><style>body{margin:24px;background:#f4f5f7;color:#20252d;font:16px/1.6 system-ui}figure{margin:24px 0}img{width:100%;display:block}a{color:#3658a7}</style><h1>Independent pin pipeline</h1><p>Download concurrency: 2. Hash concurrency: 1. The slow source waits for a release signal while the fast pin completes its vendorHash. The running frame shows the fast pin completed while the slow source is still downloading.</p><p>Releasing the slow source saves the pins file. q, Esc, and Ctrl+C exit with code 130, restore the terminal, and leave the file unchanged. Screenshots replay real PTY frames from a controlled Nix backend; no network downloads are performed.</p>'''+pictures+'<p><a href="results.json">Check results</a></p></html>')


if __name__ == '__main__':
    main()
