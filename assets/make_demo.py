#!/usr/bin/env python3
"""Render assets/demo.gif from the commands and outputs of the live Windows test.

Every command and output line below is copied from docs/windows-test.md (the
email is masked). This draws them as a terminal animation; it is not a screen
recording. Needs Pillow:  python assets/make_demo.py
"""

import pathlib
from PIL import Image, ImageDraw, ImageFont

W, H = 1000, 470
BG, PANE, BORDER = (13, 17, 23), (22, 27, 34), (48, 54, 61)
FG, DIM, GREEN, BLUE, YELLOW = (201, 209, 217), (139, 148, 158), (126, 231, 135), (121, 192, 255), (227, 179, 65)


def font(size):
    for name in ["consola.ttf", "Consolas.ttf", "DejaVuSansMono.ttf", "Menlo.ttc", "LiberationMono-Regular.ttf"]:
        try:
            return ImageFont.truetype(name, size)
        except OSError:
            continue
    return ImageFont.load_default()


F, FT = font(15), font(13)
LH = 21

# (pane, kind, text): kind "cmd" is typed out, "out" appears at once, "pause" waits.
SCRIPT = [
    ("L", "cmd", "keepapart list"),
    ("L", "out", "  NAME      TOOLS   LOGIN FILE   LAST USED"),
    ("L", "out", "  personal  claude  claude:yes   never"),
    ("L", "out", "  work      claude  claude:yes   2m ago"),
    ("L", "pause", 8),
    ("L", "cmd", 'keepapart run work -- -p "Reply with exactly: hello from the work profile"'),
    ("R", "cmd", 'keepapart run personal -- -p "Reply with exactly: hello from the personal profile"'),
    ("L", "pause", 10),
    ("L", "out!", "hello from the work profile"),
    ("R", "out!", "hello from the personal profile"),
    ("L", "pause", 10),
    ("R", "cmd", "keepapart run personal -- auth logout"),
    ("R", "out", "Successfully logged out from your Anthropic account."),
    ("L", "cmd", "keepapart run work -- auth status --text"),
    ("L", "out", "Login method: Claude Pro account"),
    ("L", "out", "Email: m***@gmail.com"),
    ("R", "cmd", "keepapart run personal -- auth status --text"),
    ("R", "out", "Not logged in. Run claude auth login to authenticate."),
    ("L", "pause", 30),
]

PANES = {"L": (16, 64, W // 2 - 8), "R": (W // 2 + 8, 64, W - 16)}
TITLES = {"L": "Terminal 1", "R": "Terminal 2"}


def wrap(text, width_px, f):
    if f.getlength("$ " + text) <= width_px:
        return [text]  # keep table spacing intact
    words, lines, cur = text.split(" "), [], ""
    for w in words:
        t = (cur + " " + w).strip()
        if f.getlength("$ " + t) > width_px and cur:
            lines.append(cur)
            cur = w
        else:
            cur = t
    lines.append(cur)
    return lines


def draw(state, cursor):
    im = Image.new("RGB", (W, H), BG)
    d = ImageDraw.Draw(im)
    d.text((16, 18), "keepapart", font=font(20), fill=FG)
    d.text((126, 22), "one machine, two accounts, kept apart  (rendered from the live Windows test log)", font=FT, fill=DIM)
    for k, (x0, y0, x1) in PANES.items():
        d.rounded_rectangle((x0, y0, x1, H - 16), radius=8, fill=PANE, outline=BORDER)
        d.text((x0 + 12, y0 + 8), TITLES[k], font=FT, fill=DIM)
        y = y0 + 34
        end = (x0 + 12, y)
        for kind, text in state[k]:
            width = x1 - x0 - 28
            if kind == "cmd":
                for i, line in enumerate(wrap(text, width, F)):
                    if i == 0:
                        d.text((x0 + 12, y), "$", font=F, fill=GREEN)
                    d.text((x0 + 28, y), line, font=F, fill=BLUE if line.startswith("keepapart") else FG)
                    end = (x0 + 30 + int(F.getlength(line)), y)
                    y += LH
            else:
                color = YELLOW if text.startswith("hello") else FG
                for line in wrap(text, width, F):
                    d.text((x0 + 12, y), line, font=F, fill=color)
                    y += LH
        if cursor == k:
            d.rectangle((end[0], end[1] + 3, end[0] + 9, end[1] + 18), fill=FG)
    return im


def main():
    state = {"L": [], "R": []}
    frames, durations = [], []

    def snap(ms, cursor=None):
        frames.append(draw(state, cursor))
        durations.append(ms)

    for pane, kind, text in SCRIPT:
        if kind == "pause":
            snap(100 * text)
        elif kind == "cmd":
            state[pane].append(("cmd", ""))
            step = max(1, len(text) // 14)
            for i in range(0, len(text) + step, step):
                state[pane][-1] = ("cmd", text[: min(i, len(text))])
                snap(45, pane)
            snap(250)
        else:
            state[pane].append(("out", text))
            snap(500 if kind == "out!" else 160)

    out = pathlib.Path(__file__).with_name("demo.gif")
    pal = [f.convert("P", palette=Image.ADAPTIVE, colors=32) for f in frames]
    pal[0].save(out, save_all=True, append_images=pal[1:], duration=durations, loop=0, optimize=True)
    print(f"wrote {out} ({len(frames)} frames, {out.stat().st_size // 1024} KB)")


if __name__ == "__main__":
    main()
