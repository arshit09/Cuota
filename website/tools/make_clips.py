"""Turns the panel's Mascot Studio GIFs into sprite sheets for the website's live panel.

From the repo root: python website/tools/make_clips.py assets/mascot website/assets/clawd
Mirrors src/animations.rs: frames at half size (the GIFs are 2x nearest-neighbour
enlargements), a delay of 0 played as a tenth of a second, and the clip's bounds (everything
any frame draws, in GIF pixels) so the page can centre it in Clawd's slot as the app does.
Each sheet holds the frames cropped to those bounds, in a grid.
"""
import json
import math
import sys
from pathlib import Path

from PIL import Image, ImageSequence

src, out = Path(sys.argv[1]), Path(sys.argv[2])
out.mkdir(parents=True, exist_ok=True)

# The panel's pools, from src/animations.rs (the error clips aren't shown on the site).
POOLS = {
    "working": [
        "thinking", "thinking-wizard-hat", "thinking-fluid",
        "using-tool", "using-tool-fluid",
        "directing-helpers", "directing-helpers-fluid",
        "reading", "reading-wizard-hat", "reading-fluid",
        "writing", "writing-sunglasses", "writing-fluid",
        "running-commands", "running-commands-sunglasses", "running-commands-fluid",
        "searching-web", "searching-web-fluid",
        "searching-code", "searching-code-fluid",
        "testing", "testing-fluid",
        "committing", "committing-fluid",
        "compacting", "compacting-fluid",
        "context-nearly-full", "context-nearly-full-fluid",
        "plan-progress", "plan-progress-fluid",
        "server-running", "server-running-fluid",
        "walking", "walking-fluid",
        "gag-hallucinate", "gag-tokens", "gag-context", "gag-prompt",
        "gag-robot", "gag-dance", "gag-buffering", "gag-deploy",
    ],
    "waiting": ["idle", "idle-wizard-hat", "idle-fluid", "waiting-for-you", "waiting-for-you-fluid", "typing", "typing-fluid", "gag-coffee"],
    "asleep": ["asleep", "asleep-nightcap", "asleep-fluid", "gag-nap"],
    "greeting": ["greeting", "greeting-crown", "greeting-fluid"],
    "done": ["done", "done-fluid"],
}
SHRINK = 2
COLUMNS = 10

clips = {}
total = 0
worst_block = 0
for name in sorted({n for pool in POOLS.values() for n in pool}):
    gif = Image.open(src / f"{name}.gif")
    assert gif.size == (480, 440), (name, gif.size)
    frames, ends, end = [], [], 0
    for frame in ImageSequence.Iterator(gif):
        rgba = frame.convert("RGBA")
        # Check the 2x enlargement claim: every 2x2 block is one colour.
        px = rgba.load()
        for y in range(0, 440, 2):
            for x in range(0, 480, 2):
                a = px[x, y]
                if a != px[x + 1, y] or a != px[x, y + 1] or a != px[x + 1, y + 1]:
                    worst_block += 1
        frames.append(rgba.resize((480 // SHRINK, 440 // SHRINK), Image.NEAREST))
        delay = frame.info.get("duration", 0) or 100
        end += delay
        ends.append(end)
    # Bounds over all frames, in half-size pixels.
    box = None
    for f in frames:
        b = f.getchannel("A").point(lambda a: 255 if a > 0 else 0).getbbox()
        if b:
            box = b if box is None else (min(box[0], b[0]), min(box[1], b[1]), max(box[2], b[2]), max(box[3], b[3]))
    box = box or (0, 0, 240, 220)
    w, h = box[2] - box[0], box[3] - box[1]
    cols = min(len(frames), COLUMNS)
    rows = math.ceil(len(frames) / cols)
    sheet = Image.new("RGBA", (cols * w, rows * h), (0, 0, 0, 0))
    for i, f in enumerate(frames):
        sheet.paste(f.crop(box), ((i % cols) * w, (i // cols) * h))
    path = out / f"{name}.png"
    sheet.save(path, optimize=True)
    total += path.stat().st_size
    clips[name] = {
        "cols": cols,
        "w": w,
        "h": h,
        # In GIF pixels, as the app keeps them.
        "bounds": [box[0] * SHRINK, box[1] * SHRINK, w * SHRINK, h * SHRINK],
        "ends": ends,
    }
    print(f"{name:30} {len(frames):3} frames {end:6} ms  {w}x{h}  {path.stat().st_size // 1024} KB")

script = (
    "// Clawd's clips for the live panel, made from assets/mascot by tools/make_clips.py: each\n"
    "// sheet's frames are cropped to the clip's bounds; `ends` are when each frame ends, in ms.\n"
    f"window.CUOTA_CLIPS = {json.dumps({'pools': POOLS, 'clips': clips}, separators=(',', ':'))};\n"
)
(out / "clips.js").write_text(script, encoding="utf-8")
print(f"{len(clips)} clips, {total // 1024} KB of sheets, blocks not 2x2-uniform: {worst_block}")
