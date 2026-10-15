#!/usr/bin/env python3
"""Guard RV presentation policy without treating image/canvas geometry as controls."""
from pathlib import Path
import re
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]


def blank(value):
    return "".join("\n" if char == "\n" else " " for char in value)


def code_only(source):
    """Keep source positions; remove Rust comments, strings and character literals."""
    out = list(source)
    i = 0
    while i < len(source):
        start = i
        if source.startswith("//", i):
            end = source.find("\n", i)
            i = len(source) if end < 0 else end
        elif source.startswith("/*", i):
            i += 2
            depth = 1
            while i < len(source) and depth:
                if source.startswith("/*", i):
                    depth += 1
                    i += 2
                elif source.startswith("*/", i):
                    depth -= 1
                    i += 2
                else:
                    i += 1
        else:
            raw = re.match(r'(?:br|r)(#*)"', source[i:])
            char = re.match(r"'(?:[^'\\\n]|\\(?:u\{[0-9a-fA-F]+\}|x[0-9a-fA-F]{2}|.))'", source[i:])
            if raw:
                close = '"' + raw[1]
                end = source.find(close, i + raw.end())
                i = len(source) if end < 0 else end + len(close)
            elif source[i] == '"':
                i += 1
                while i < len(source):
                    if source[i] == "\\":
                        i += 2
                    elif source[i] == '"':
                        i += 1
                        break
                    else:
                        i += 1
            elif char:
                i += char.end()
            else:
                i += 1
                continue
        out[start:i] = blank(source[start:i])
    return "".join(out)


def closing(code, start):
    pairs = {"(": ")", "[": "]", "{": "}"}
    stack = [pairs[code[start]]]
    for pos in range(start + 1, len(code)):
        char = code[pos]
        if char in pairs:
            stack.append(pairs[char])
        elif char in ")]}" and stack and char == stack[-1]:
            stack.pop()
            if not stack:
                return pos
    return len(code) - 1


def production_code(source):
    code = code_only(source)
    # Test-only methods can precede production methods in the same impl.
    for match in list(re.finditer(r"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]", code)):
        tail = re.search(r"[;{]", code[match.end():])
        if not tail:
            continue
        start = match.end() + tail.start()
        end = closing(code, start) + 1 if code[start] == "{" else start + 1
        code = code[:match.start()] + blank(code[match.start():end]) + code[end:]
    return code


def violations(source):
    code = production_code(source)
    found = []
    rules = [
        (r"\b(?:rgb|rgba|hsl|hsla)\s*\(", "颜色应取自 theme 语义色"),
        (r"\.\s*(?:text_size|font_family|font_weight|set_rem_size)\s*\(", "字体/字号使用主题与标准文字角色"),
        (r"\.\s*text_(?:base|lg|xl|[2-9]xl)\s*\(", "页面文字使用正文/辅助角色；Dialog 标题由组件管理"),
        (r"\.\s*(?:mono_)?font_size\s*=", "字体基准只能在 theme 中设置"),
        (r"\.\s*rounded_(?:sm|md|lg)\s*\(", "自定义容器圆角应读取 theme radius"),
        (r"\bfn\s+(?:tool_btn|compact_button|function_icon_button|primary_button|secondary_button|danger_button)\s*\(",
         "复用标准 Button 或 ui 公共组合，避免重复样式封装"),
    ]
    for pattern, message in rules:
        found.extend((match.start(), message) for match in re.finditer(pattern, code))
    for match in re.finditer(r'\.(?:label|child)\(\s*"[+＋×✕←→↑↓↻○ϟ⇅⋯]"\s*\)', source):
        if code[match.start()] == ".":
            found.append((match.start(), "操作图标使用 Lucide Icon，不能用文字字符代替"))
    # Walk only the direct control chain. Nested image/canvas containers may size freely.
    for match in re.finditer(r"\b(Button|Input|Select|Checkbox|Switch|Tab)::new\s*\(", code):
        pos = closing(code, match.end() - 1) + 1
        while method := re.match(r"\s*\.\s*(\w+)\s*\(", code[pos:]):
            op = pos + method.end() - 1
            end = closing(code, op)
            name = method[1]
            args = code[op + 1:end]
            if (name == "h" or (match[1] == "Button" and name == "w")) and re.search(r"\bpx\s*\(\s*\d", args):
                found.append((pos + method.start(), "控件尺寸使用组件默认值/标准 Size；内容区域的尺寸可自行布局"))
            if re.fullmatch(r"p[xytrbl]?_\w+", name):
                found.append((pos + method.start(), "控件内边距由组件统一管理"))
            pos = end + 1
    return [(source.count("\n", 0, pos) + 1, message) for pos, message in sorted(set(found))]


class ScannerTests(unittest.TestCase):
    def test_comments_strings_and_raw_strings_are_not_styles(self):
        self.assertEqual(violations('// rgb(1)\nlet x = r#".text_size(px(4.))"#; /* .rounded_md() */'), [])

    def test_test_items_do_not_hide_later_production(self):
        errors = violations('#[cfg(test)] fn f() { rgb(1); }\nfn view() { rgb(2); }')
        self.assertEqual([line for line, _ in errors], [2])

    def test_control_chain_checks_padding_and_height(self):
        self.assertEqual(len(violations('Button::new("x").ghost().h(px(19.)).px_0().label("x");')), 2)

    def test_nested_geometry_is_not_control_geometry(self):
        self.assertEqual(violations('Button::new("x").child(div().h(px(80.)));'), [])

    def test_nested_controls_and_characters(self):
        self.assertEqual(len(violations('let c = \'{\'; div().child(Input::new(&state).h(px(17.)));')), 1)


def main():
    if "--self-test" in sys.argv:
        unittest.main(argv=[sys.argv[0]])
        return
    errors = []
    for path in sorted((ROOT / "crates/rv-app/src").rglob("*.rs")):
        if path.name == "theme.rs" or path.stem.endswith("tests"):
            continue
        errors.extend(f"{path.relative_to(ROOT)}:{line}: {message}" for line, message in violations(path.read_text()))
    if errors:
        print("\n".join(errors))
        raise SystemExit(1)
    print("UI style policy passed: theme colors, typography, radii and default control sizing")


if __name__ == "__main__":
    main()
