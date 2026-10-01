"""检查静态产物中的入口跳转、站内链接、资源与锚点，包括 GitHub Pages 子路径。"""

import os
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import unquote, urljoin, urlsplit


class Page(HTMLParser):
    def __init__(self, path):
        super().__init__()
        self.ids = set()
        self.links = []
        self.lang = None
        self.redirect = None
        self.feed(path.read_text())

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if tag == "html":
            self.lang = attrs.get("lang")
        if tag == "meta" and attrs.get("http-equiv", "").lower() == "refresh":
            self.redirect = attrs.get("content")
        if attrs.get("id"):
            self.ids.add(attrs["id"])
        for name in ("href", "src"):
            if attrs.get(name):
                self.links.append(attrs[name])


def target_file(root, path):
    path = root / path.lstrip("/")
    for candidate in (path, path.with_suffix(".html"), path / "index.html"):
        if candidate.is_file():
            return candidate
    return None


def main():
    root = Path(__file__).resolve().parents[1] / ".output/public"
    base = "/" + os.environ.get("NUXT_APP_BASE_URL", "/").strip("/")
    base = base.rstrip("/") + "/"
    pages = {path: Page(path) for path in root.rglob("*.html")}
    if not pages:
        raise SystemExit("没有静态 HTML；先执行 pnpm docs:build")
    failures = []
    redirects = {
        "zh-CN": "zh-CN/getting-started/installation",
        "zh-CN/getting-started": "zh-CN/getting-started/installation",
        "zh-CN/guides": "zh-CN/guides/updating",
        "zh-CN/reference": "zh-CN/reference/cli",
        "zh-CN/troubleshooting": "zh-CN/troubleshooting/common-errors",
    }
    for route in redirects:
        if root / f"{route}.html" not in pages:
            failures.append(f"缺少 /{route} 入口跳转页")
    checked = 0
    for path, page in pages.items():
        route = path.relative_to(root).as_posix()
        route = "/" if route == "index.html" else route.removesuffix(".html")
        if route in redirects:
            destination = base + redirects[route]
            if page.redirect != f"0; url={destination}":
                failures.append(f"{route}: 入口应跳转到 {destination}")
            page.links.append(destination)
        elif page.lang != "zh-CN":
            failures.append(f"{path.name}: 文档语言应为 zh-CN，得到 {page.lang}")
        if path.name not in ("200.html", "404.html") and not (
            route == "zh-CN" or route.startswith("zh-CN/")
        ):
            failures.append(f"{route}: 文档路由应使用 zh-CN 语言前缀")
        source = "https://docs.invalid" + base + route.lstrip("/")
        for link in page.links:
            original = urlsplit(link)
            if original.scheme or original.netloc:
                continue
            url = urlsplit(urljoin(source, link))
            decoded = unquote(url.path)
            if not decoded.startswith(base):
                failures.append(f"{route}: 缺少站点前缀 {base}: {link}")
                continue
            target = target_file(root, decoded[len(base):])
            if not target:
                failures.append(f"{route}: 目标不存在: {link}")
            elif url.fragment and target.suffix == ".html":
                document = pages.setdefault(target, Page(target))
                if unquote(url.fragment) not in document.ids:
                    failures.append(f"{route}: 锚点不存在: {link}")
            checked += 1
    if failures:
        raise SystemExit("\n".join(failures))
    print(f"通过：{len(pages)} 个 HTML，{checked} 个站内链接／资源，base={base}")


if __name__ == "__main__":
    main()
