#!/usr/bin/env python3
"""Converts `tmux capture-pane -e -p` output on stdin into a styled HTML terminal window."""
import html
import re
import sys

BASE16 = [(0, 0, 0), (205, 49, 49), (13, 188, 121), (229, 229, 16), (36, 114, 200),
          (188, 63, 188), (17, 168, 205), (229, 229, 229), (102, 102, 102), (241, 76, 76),
          (35, 209, 139), (245, 245, 67), (59, 142, 234), (214, 112, 214), (41, 184, 219),
          (255, 255, 255)]


def c256(n):
    if n < 16:
        return BASE16[n]
    if n < 232:
        n -= 16
        lv = [0, 95, 135, 175, 215, 255]
        return lv[n // 36], lv[n // 6 % 6], lv[n % 6]
    v = 8 + 10 * (n - 232)
    return v, v, v


def convert(data, title):
    out = []
    fg = bg = None
    bold = dim = ul = it = rev = False

    def style():
        f, b = fg, bg
        if rev:
            f, b = (bg or (13, 17, 23)), (fg or (201, 209, 217))
        s = []
        if f:
            s.append("color:rgb(%d,%d,%d)" % f)
        if b:
            s.append("background:rgb(%d,%d,%d)" % b)
        if bold:
            s.append("font-weight:bold")
        if dim:
            s.append("opacity:.7")
        if ul:
            s.append("text-decoration:underline")
        if it:
            s.append("font-style:italic")
        return ";".join(s)

    token = re.compile(r"\x1b\[([0-9;]*)m|\x1b\[[0-9;?]*[A-Za-z]|([^\x1b]+)")
    for m in token.finditer(data):
        if m.group(2) is not None:
            out.append('<span style="%s">%s</span>' % (style(), html.escape(m.group(2))))
            continue
        if m.group(1) is None:
            continue
        p = [int(x) if x else 0 for x in m.group(1).split(";")]
        i = 0
        while i < len(p):
            x = p[i]
            if x == 0:
                fg = bg = None
                bold = dim = ul = it = rev = False
            elif x == 1:
                bold = True
            elif x == 2:
                dim = True
            elif x == 3:
                it = True
            elif x == 4:
                ul = True
            elif x == 7:
                rev = True
            elif x == 22:
                bold = dim = False
            elif x == 23:
                it = False
            elif x == 24:
                ul = False
            elif x == 27:
                rev = False
            elif 30 <= x <= 37:
                fg = c256(x - 30)
            elif 90 <= x <= 97:
                fg = c256(x - 90 + 8)
            elif 40 <= x <= 47:
                bg = c256(x - 40)
            elif 100 <= x <= 107:
                bg = c256(x - 100 + 8)
            elif x == 39:
                fg = None
            elif x == 49:
                bg = None
            elif x in (38, 48):
                if p[i + 1] == 2:
                    col = tuple(p[i + 2:i + 5])
                    i += 4
                else:
                    col = c256(p[i + 2])
                    i += 2
                if x == 38:
                    fg = col
                else:
                    bg = col
            i += 1
    body = "".join(out)
    return f"""<!doctype html><html><head><meta charset="utf-8"><style>
body{{margin:0;padding:28px;background:#1b1f27;display:inline-block}}
.win{{background:#0d1117;border-radius:10px;overflow:hidden;box-shadow:0 8px 30px rgba(0,0,0,.5);border:1px solid #30363d}}
.bar{{height:30px;background:#161b22;display:flex;align-items:center;padding:0 12px;gap:8px;border-bottom:1px solid #30363d}}
.dot{{width:12px;height:12px;border-radius:50%}}
.title{{color:#8b949e;font:12px -apple-system,Helvetica,sans-serif;margin-left:10px}}
pre{{margin:0;padding:12px 14px;font:13px/1.25 Menlo,Monaco,monospace;color:#c9d1d9}}
</style></head><body><div class="win"><div class="bar">
<span class="dot" style="background:#ff5f56"></span><span class="dot" style="background:#ffbd2e"></span><span class="dot" style="background:#27c93f"></span>
<span class="title">{html.escape(title)}</span></div><pre>{body}</pre></div></body></html>"""


if __name__ == "__main__":
    print(convert(sys.stdin.read(), sys.argv[1] if len(sys.argv) > 1 else "pane"))
