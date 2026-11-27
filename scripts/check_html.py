# SPDX-License-Identifier: MPL-2.0

from html.parser import HTMLParser
from pathlib import Path
import json
import re
import sys

ROOT = Path(__file__).resolve().parent.parent
TARGET = ROOT / "dist" / "index.html"
SCRIPT = ROOT / "dist" / "main.js"
CONFIG = ROOT / "src-tauri" / "tauri.conf.json"
VOID = {"meta", "link", "input", "br", "img", "hr", "source", "area", "base", "col", "embed", "param", "track", "wbr"}
REQUIRED_IDS = [
    "state", "account", "lastCheck", "lastLogin", "message",
    "setup", "reconnect", "reset", "autostart", "hide", "quit",
    "credentials", "accountInput", "passwordInput", "notice",
]


class Checker(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.stack = []
        self.ids = []
        self.errors = []

    def handle_starttag(self, tag, attrs):
        attributes = dict(attrs)
        if "id" in attributes:
            self.ids.append(attributes["id"])
        if tag not in VOID and not attributes.get("data-self-closing"):
            self.stack.append(tag)

    def handle_startendtag(self, tag, attrs):
        attributes = dict(attrs)
        if "id" in attributes:
            self.ids.append(attributes["id"])

    def handle_endtag(self, tag):
        if tag in VOID:
            return
        if not self.stack:
            self.errors.append(f"多余的结束标签 </{tag}>")
            return
        top = self.stack.pop()
        if top != tag:
            self.errors.append(f"标签不匹配：<{top}> 被 </{tag}> 关闭")


def main():
    if not TARGET.exists():
        print(f"缺少 {TARGET}")
        return 1

    raw = TARGET.read_text(encoding="utf-8")
    errors = []

    checker = Checker()
    checker.feed(raw)
    checker.close()
    errors.extend(checker.errors)
    if checker.stack:
        errors.append(f"未闭合标签：{checker.stack}")

    if (raw.count("<title>") , raw.count("</title>")) != (1, 1):
        errors.append("title 标签数量异常")

    # HTMLParser 会把引号内的 < > 当作普通文本，但浏览器也可能产生歧义，直接拒绝。
    for index, line in enumerate(raw.splitlines(), 1):
        for token in ('="</', '="<', '>"'):
            if token in line:
                errors.append(f"第 {index} 行属性值含可疑标记：{token}")
                break

    duplicates = sorted({i for i in checker.ids if checker.ids.count(i) > 1})
    if duplicates:
        errors.append(f"重复 id：{duplicates}")

    missing = [i for i in REQUIRED_IDS if i not in checker.ids]
    if missing:
        errors.append(f"缺少 id：{missing}")

    if SCRIPT.exists():
        js = SCRIPT.read_text(encoding="utf-8")
        # 没有打包器时，裸模块说明符无法在 WebView 中解析。
        bare = re.findall(r'\bfrom\s+["\'](@tauri-apps/[^"\']+)["\']', js)
        if bare:
            errors.append(f"main.js 含未打包的裸模块导入：{bare}")
        if "window.__TAURI__" not in js:
            errors.append("main.js 未使用 window.__TAURI__ 全局 API")
    else:
        errors.append(f"缺少 {SCRIPT}")

    if CONFIG.exists():
        config = json.loads(CONFIG.read_text(encoding="utf-8"))
        if config.get("app", {}).get("withGlobalTauri") is not True:
            errors.append("tauri.conf.json 缺少 app.withGlobalTauri: true")
    else:
        errors.append(f"缺少 {CONFIG}")

    if errors:
        print("HTML 校验失败：")
        for error in errors:
            print(f"- {error}")
        return 1

    print(f"HTML OK: {TARGET.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
