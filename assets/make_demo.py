#!/usr/bin/env python3
"""Render assets/demo.gif (and demo.mp4 when ffmpeg is available).

Every command and every line of output below was produced by running keepapart
for real on Windows 11 (docs/windows-test.md has the full log). Only three
things are changed: the email is masked, temp folders are shown as the paths a
normal install uses (~\\.keepapart, ~\\code\\acme-app), and waiting time is
shortened. This draws them as an animation; it is not a screen recording.

    python assets/make_demo.py
"""

import pathlib
import shutil
import subprocess

from PIL import Image, ImageDraw, ImageFont

W, H = 1280, 720
BG = (13, 17, 23)
PANE, BORDER, BAR = (22, 27, 34), (48, 54, 61), (33, 38, 45)
FG, DIM = (230, 237, 243), (139, 148, 158)
GREEN, BLUE, YELLOW, RED, PURPLE = (126, 231, 135), (121, 192, 255), (240, 200, 90), (255, 123, 114), (210, 168, 255)


def load(names, size):
    for n in names:
        try:
            return ImageFont.truetype(n, size)
        except OSError:
            pass
    return ImageFont.load_default()


MONO = ["CascadiaMono.ttf", "consola.ttf", "Menlo.ttc", "DejaVuSansMono.ttf"]
SANS = ["segoeui.ttf", "Helvetica.ttc", "DejaVuSans.ttf"]
SANS_B = ["segoeuib.ttf", "Helvetica.ttc", "DejaVuSans-Bold.ttf"]
F = load(MONO, 23)
F_SMALL = load(SANS, 16)
F_TITLE = load(SANS_B, 30)
F_BIG = load(SANS_B, 76)
F_SUB = load(SANS, 30)
F_BODY = load(SANS, 26)
LH = 33

FOOT = "Real commands and output from a Windows 11 session, rendered. Email masked, waits shortened."

frames, durations = [], []


def add(img, ms):
    frames.append(img)
    durations.append(ms)


def centered(d, y, text, font, fill):
    d.text(((W - d.textlength(text, font=font)) / 2, y), text, font=font, fill=fill)


def wrap(text, width):
    """Split a line to fit `width` px, keeping its indent; table rows that fit stay as-is."""
    if F.getlength(text) <= width:
        return [text]
    indent = " " * (len(text) - len(text.lstrip(" ")))
    out, cur = [], indent
    for w in text.strip(" ").split(" "):
        t = cur + w if cur.strip() == "" else cur + " " + w
        if F.getlength(t) > width and cur.strip():
            out.append(cur)
            cur = indent + "  " + w
        else:
            cur = t
    return out + [cur]


class Pane:
    def __init__(self, box, title):
        self.box, self.title, self.lines = box, title, []  # lines: (kind, text)

    def draw(self, d, cursor=False):
        x0, y0, x1, y1 = self.box
        d.rounded_rectangle(self.box, radius=10, fill=PANE, outline=BORDER)
        d.rounded_rectangle((x0, y0, x1, y0 + 34), radius=10, fill=BAR)
        d.rectangle((x0, y0 + 24, x1, y0 + 34), fill=BAR)
        for i, c in enumerate([(255, 95, 86), (255, 189, 46), (39, 201, 63)]):
            d.ellipse((x0 + 14 + i * 20, y0 + 11, x0 + 26 + i * 20, y0 + 23), fill=c)
        d.text((x0 + 84, y0 + 7), self.title, font=F_SMALL, fill=DIM)
        y, width = y0 + 48, x1 - x0 - 46
        end = (x0 + 16, y)
        for kind, text in self.lines:
            for i, line in enumerate(wrap(text, width)):
                if kind == "cmd":
                    if i == 0:
                        d.text((x0 + 16, y), "❯", font=F, fill=GREEN)
                    d.text((x0 + 38, y), line, font=F, fill=BLUE)
                    end = (x0 + 40 + int(F.getlength(line)), y)
                else:
                    color = {"ok": GREEN, "hi": YELLOW, "bad": RED, "out": FG, "dim": DIM}[kind]
                    d.text((x0 + 16, y), line, font=F, fill=color)
                    end = (x0 + 16, y + LH)
                y += LH
        if cursor:
            d.rectangle((end[0], end[1] + 4, end[0] + 10, end[1] + 23), fill=FG)


def scene_frame(title, panes, cursor_pane=None):
    im = Image.new("RGB", (W, H), BG)
    d = ImageDraw.Draw(im)
    d.text((40, 26), "keepapart", font=F_TITLE, fill=FG)
    d.text((40 + d.textlength("keepapart  ", font=F_TITLE), 31), title, font=load(SANS, 26), fill=PURPLE)
    for p in panes:
        p.draw(d, cursor=(p is cursor_pane))
    centered(d, H - 30, FOOT, F_SMALL, DIM)
    return im


def type_cmd(title, panes, pane, text, step_ms=28, chunk=4):
    pane.lines.append(("cmd", ""))
    for i in range(0, len(text) + chunk, chunk):
        pane.lines[-1] = ("cmd", text[: min(i, len(text))])
        add(scene_frame(title, panes, pane), step_ms)
    add(scene_frame(title, panes), 300)


def out(title, panes, pane, kind, text, ms=120):
    pane.lines.append((kind, text))
    add(scene_frame(title, panes), ms)


def card(lines, ms):
    im = Image.new("RGB", (W, H), BG)
    d = ImageDraw.Draw(im)
    y = sum(h for _, _, _, h in lines)
    y = (H - y) / 2
    for text, font, fill, h in lines:
        centered(d, y, text, font, fill)
        y += h
    add(im, ms)


def main():
    # 1. Title
    card(
        [
            ("keepapart", F_BIG, FG, 110),
            ("One machine, many AI coding accounts.", F_SUB, FG, 44),
            ("Work, personal, clients, kept apart.", F_SUB, PURPLE, 70),
            ("Claude Code · Codex · Gemini · Grok · Kimi · Qwen · Goose · Crush · Cline · Continue · Aider · OpenCode", load(SANS, 22), FG, 46),
            ("Windows · macOS · Linux     open source (MIT)", F_BODY, DIM, 40),
        ],
        2600,
    )

    # 2. Side by side
    t = "two accounts, side by side"
    left = Pane((40, 90, 634, 560), "Terminal 1")
    right = Pane((646, 90, 1240, 560), "Terminal 2")
    ps = [left, right]
    add(scene_frame(t, ps), 500)
    type_cmd(t, ps, left, 'keepapart run work -- --model sonnet -p "Reply with exactly: hello from the work profile"')
    type_cmd(t, ps, right, 'keepapart run personal -- --model sonnet -p "Reply with exactly: hello from the personal profile"')
    add(scene_frame(t, ps), 900)
    out(t, ps, left, "hi", "hello from the work profile", 150)
    out(t, ps, right, "hi", "hello from the personal profile", 150)
    out(t, ps, left, "dim", "(5.3 s)", 0)
    out(t, ps, right, "dim", "(5.7 s, at the same time)", 2400)

    # 3. Separate logins
    t = "each profile has its own login"
    left = Pane((40, 90, 634, 560), "Terminal 1")
    right = Pane((646, 90, 1240, 560), "Terminal 2")
    ps = [left, right]
    add(scene_frame(t, ps), 300)
    type_cmd(t, ps, left, "keepapart run work --tool codex -- login status")
    out(t, ps, left, "ok", "Logged in using ChatGPT", 500)
    type_cmd(t, ps, right, "keepapart run personal --tool codex -- login status")
    out(t, ps, right, "bad", "Not logged in", 700)
    type_cmd(t, ps, left, "keepapart run work -- auth status --text")
    out(t, ps, left, "out", "Login method: Claude Pro account")
    out(t, ps, left, "out", "Email: m***@gmail.com", 400)
    type_cmd(t, ps, right, "keepapart run personal -- auth status --text")
    out(t, ps, right, "out", "Login method: Claude Pro account")
    out(t, ps, right, "out", "Email: m***@gmail.com", 2600)

    # 4. Per-folder profile
    t = "a client repo always uses the client account"
    p = Pane((40, 90, 1240, 640), "~/code/acme-app")
    ps = [p]
    add(scene_frame(t, ps), 300)
    type_cmd(t, ps, p, "keepapart use client-acme")
    out(t, ps, p, "out", 'Wrote ~\\code\\acme-app\\.keepapart. `keepapart run` in this folder (and below) uses "client-acme".', 700)
    type_cmd(t, ps, p, "keepapart status")
    out(t, ps, p, "ok", "Profile: client-acme (from ~\\code\\acme-app\\.keepapart)")
    out(t, ps, p, "out", "  claude  CLAUDE_CONFIG_DIR=~\\.keepapart\\profiles\\client-acme\\claude")
    out(t, ps, p, "out", "  shared  claude:CLAUDE.md, claude:agents, claude:commands, claude:output-styles, claude:skills", 900)
    type_cmd(t, ps, p, "keepapart list")
    out(t, ps, p, "out", "  NAME         TOOLS         LOGIN FILE                LAST USED")
    out(t, ps, p, "ok", "* client-acme  claude        claude:no                 never")
    out(t, ps, p, "out", "  work         claude+codex  claude:no codex:no        never", 2800)

    # 5. Trust
    card(
        [
            ("Your tokens are never touched", F_TITLE, FG, 70),
            ("Each profile gives each AI CLI its own folder (CLAUDE_CONFIG_DIR, GEMINI_CLI_HOME, ...).", F_BODY, FG, 44),
            ("You log in with the official CLI. keepapart never reads a token.", F_BODY, FG, 44),
            ("No network calls. No telemetry.", F_BODY, FG, 44),
            ("No rate-limit hopping: it keeps accounts apart, it doesn't rotate them.", F_BODY, PURPLE, 44),
        ],
        3800,
    )

    # 6. Install
    card(
        [
            ("Install", F_TITLE, FG, 66),
            ("scoop bucket add keepapart https://github.com/moelsaka01/keepapart", F, BLUE, 34),
            ("scoop install keepapart/keepapart", F, BLUE, 52),
            ("brew tap moelsaka01/keepapart https://github.com/moelsaka01/keepapart", F, BLUE, 34),
            ("brew install keepapart", F, BLUE, 52),
            ("cargo install keepapart", F, BLUE, 72),
            ("github.com/moelsaka01/keepapart", F_TITLE, YELLOW, 40),
        ],
        4600,
    )

    here = pathlib.Path(__file__).parent
    gif = here / "demo.gif"
    # One palette for all frames, built from the last frame of every scene,
    # so every color (green, red, yellow, the window dots) survives.
    holds = [f for f, ms in zip(frames, durations) if ms >= 2000]
    sheet = Image.new("RGB", (W, H * len(holds)))
    for i, f in enumerate(holds):
        sheet.paste(f, (0, H * i))
    pal = sheet.quantize(colors=128, method=Image.Quantize.MEDIANCUT)
    q = [f.quantize(palette=pal, dither=Image.Dither.NONE) for f in frames]
    q[0].save(gif, save_all=True, append_images=q[1:], duration=durations, loop=0, optimize=True, disposal=1)
    total = sum(durations) / 1000
    print(f"wrote {gif} ({len(frames)} frames, {total:.1f}s, {gif.stat().st_size // 1024} KB)")

    if shutil.which("ffmpeg"):
        tmp = here / "_frames"
        tmp.mkdir(exist_ok=True)
        lines = []
        for i, (f, ms) in enumerate(zip(frames, durations)):
            name = f"{i:04d}.png"
            f.save(tmp / name)
            lines += [f"file '{name}'", f"duration {max(ms, 20) / 1000:.3f}"]
        lines.append(f"file '{len(frames) - 1:04d}.png'")
        (tmp / "list.txt").write_text("\n".join(lines) + "\n")
        mp4 = here / "demo.mp4"
        subprocess.run(
            ["ffmpeg", "-y", "-loglevel", "error", "-f", "concat", "-safe", "0", "-i", str(tmp / "list.txt"),
             "-vf", "fps=30,format=yuv420p", "-c:v", "libx264", "-crf", "18", "-movflags", "+faststart", str(mp4)],
            check=True,
        )
        shutil.rmtree(tmp)
        print(f"wrote {mp4} ({mp4.stat().st_size // 1024} KB)")


if __name__ == "__main__":
    main()
