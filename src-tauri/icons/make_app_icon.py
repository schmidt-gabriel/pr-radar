#!/usr/bin/env python3
"""Rasterize the PR Radar application icon.

Companion to make_tray_icon.py: same radar motif, but in colour and on the
Big Sur squircle rather than a flat template. No image libraries are used --
this writes RGBA PNGs directly and lets `iconutil` assemble the .icns.

    python3 make_app_icon.py .
"""

import math
import os
import struct
import subprocess
import sys
import zlib

# --- palette, taken from the design doc's tokens -------------------------
BG_TOP = (0x20, 0x26, 0x34)
BG_BOTTOM = (0x0B, 0x0E, 0x13)
RING = (0x7A, 0xA2, 0xFF)      # --blue
BEAM = (0xC2, 0xD5, 0xFF)
BLIP = (0x5E, 0xD3, 0x6B)      # --green-bright
BLIP_ALERT = (0xFF, 0x8A, 0x82)  # --red-bright

# --- geometry, in unit coordinates over the full canvas ------------------
# Apple's grid: the squircle covers ~80% of the canvas, the rest is padding.
SQ_INSET = 0.0977
SQ_N = 5.0                      # superellipse exponent approximating the squircle

BEAM_ANGLE = 64.0
GLOW = 3.4                      # glow radius, in multiples of the dot radius

# Three rings and two contacts turn to mush below about 64px, so small sizes get
# their own simplified geometry: fewer rings, heavier strokes, one contact.
# This is the same trick a hand-drawn icon set uses, just parameterised.
DETAILED = {
    "rings": ((0.300, 0.0150), (0.205, 0.0125), (0.110, 0.0110)),
    "beam_stroke": 0.0130,
    "sweep_span": 96.0,
    "sweep_max": 0.42,
    "blips": ((0.232, 22.0, 0.0250, BLIP), (0.148, 143.0, 0.0165, BLIP_ALERT)),
    "ring_alpha": (0.92, 0.62),
}

SIMPLE = {
    "rings": ((0.310, 0.0330), (0.170, 0.0260)),
    "beam_stroke": 0.0300,
    "sweep_span": 104.0,
    "sweep_max": 0.52,
    "blips": ((0.238, 20.0, 0.0450, BLIP),),
    "ring_alpha": (1.0, 0.78),
}


def params_for(size):
    return SIMPLE if size <= 64 else DETAILED


def lerp(a, b, t):
    return a + (b - a) * t


def inside_squircle(x, y):
    """Superellipse test in canvas coordinates."""
    half = 0.5 - SQ_INSET
    u, v = abs(x - 0.5) / half, abs(y - 0.5) / half
    return (u ** SQ_N + v ** SQ_N) <= 1.0


def background(x, y):
    """Vertical gradient plus a soft top-left sheen."""
    t = (y - SQ_INSET) / (1.0 - 2 * SQ_INSET)
    t = min(max(t, 0.0), 1.0)
    col = [lerp(BG_TOP[i], BG_BOTTOM[i], t ** 0.85) for i in range(3)]

    sheen = math.exp(-(((x - 0.34) ** 2 + (y - 0.28) ** 2) / 0.075))
    return [min(255.0, c + 26.0 * sheen) for c in col]


def over(dst, src, alpha):
    if alpha <= 0.0:
        return dst
    a = min(1.0, alpha)
    return [dst[i] * (1.0 - a) + src[i] * a for i in range(3)]


def shade(x, y, p):
    """Composited colour at one sample point. Returns (r, g, b) or None."""
    if not inside_squircle(x, y):
        return None

    col = background(x, y)
    dx, dy = x - 0.5, 0.5 - y
    d = math.hypot(dx, dy)
    ang = math.degrees(math.atan2(dy, dx)) % 360.0

    outer = p["rings"][0][0]

    # Sweep tail: brightest at the beam, fading backwards.
    if d <= outer:
        delta = (BEAM_ANGLE - ang) % 360.0
        if delta <= p["sweep_span"]:
            fade = (1.0 - delta / p["sweep_span"]) ** 1.6
            edge = 1.0 - (d / outer) ** 3        # soften where it meets the rim
            col = over(col, RING, p["sweep_max"] * fade * edge)

    # Rings: the outer one carries the silhouette, the rest are support.
    for i, (r, s) in enumerate(p["rings"]):
        if abs(d - r) <= s / 2:
            col = over(col, RING, p["ring_alpha"][0 if i == 0 else 1])

    # Beam.
    rad = math.radians(BEAM_ANGLE)
    ux, uy = math.cos(rad), math.sin(rad)
    along = dx * ux + dy * uy
    if 0.0 <= along <= outer:
        if abs(-dx * uy + dy * ux) <= p["beam_stroke"] / 2:
            col = over(col, BEAM, 0.95)

    # Contacts, each with a soft glow.
    for br, bang, bdot, bcol in p["blips"]:
        rr = math.radians(bang)
        bx, by = br * math.cos(rr), br * math.sin(rr)
        dist = math.hypot(dx - bx, dy - by)
        if dist <= bdot * GLOW:
            col = over(col, bcol, 0.5 * math.exp(-((dist / (bdot * 1.5)) ** 2)))
        if dist <= bdot:
            col = over(col, bcol, 1.0)

    return col


def render(size, ss):
    rows = []
    step = 1.0 / (size * ss)
    n = ss * ss
    p = params_for(size)
    for py in range(size):
        row = bytearray()
        for px in range(size):
            acc_r = acc_g = acc_b = acc_a = 0.0
            for sy in range(ss):
                for sx in range(ss):
                    u = (px * ss + sx + 0.5) * step
                    v = (py * ss + sy + 0.5) * step
                    c = shade(u, v, p)
                    if c is not None:
                        acc_r += c[0]
                        acc_g += c[1]
                        acc_b += c[2]
                        acc_a += 1.0
            if acc_a == 0.0:
                row += b"\x00\x00\x00\x00"
            else:
                # Averaged premultiplied, converted back to straight alpha.
                row += bytes(
                    (
                        int(round(acc_r / acc_a)),
                        int(round(acc_g / acc_a)),
                        int(round(acc_b / acc_a)),
                        int(round(255 * acc_a / n)),
                    )
                )
        rows.append(bytes(row))
    return rows


def png_bytes(size, ss):
    rows = render(size, ss)
    raw = b"".join(b"\x00" + r for r in rows)

    def chunk(tag, data):
        return (
            struct.pack(">I", len(data))
            + tag
            + data
            + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
        )

    out = b"\x89PNG\r\n\x1a\n"
    out += chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
    out += chunk(b"IDAT", zlib.compress(raw, 9))
    out += chunk(b"IEND", b"")
    return out


def ss_for(size):
    """Keep total sample count sane; big canvases need less help."""
    return 4 if size <= 128 else 2


def write_ico(path, png):
    """Single-image ICO wrapping a 256px PNG, which modern Windows accepts."""
    header = struct.pack("<HHH", 0, 1, 1)
    entry = struct.pack("<BBBBHHII", 0, 0, 0, 0, 1, 32, len(png), 22)
    with open(path, "wb") as f:
        f.write(header + entry + png)


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else "."
    cache = {}

    def get(size):
        if size not in cache:
            cache[size] = png_bytes(size, ss_for(size))
            print(f"  rendered {size}x{size}")
        return cache[size]

    iconset = os.path.join(out, "icon.iconset")
    os.makedirs(iconset, exist_ok=True)

    # iconutil expects exactly these names.
    for base in (16, 32, 128, 256, 512):
        with open(os.path.join(iconset, f"icon_{base}x{base}.png"), "wb") as f:
            f.write(get(base))
        with open(os.path.join(iconset, f"icon_{base}x{base}@2x.png"), "wb") as f:
            f.write(get(base * 2))

    # The sizes tauri.conf.json lists directly.
    for name, size in (
        ("32x32.png", 32),
        ("128x128.png", 128),
        ("128x128@2x.png", 256),
        ("icon.png", 512),
    ):
        with open(os.path.join(out, name), "wb") as f:
            f.write(get(size))

    write_ico(os.path.join(out, "icon.ico"), get(256))

    subprocess.run(
        ["iconutil", "-c", "icns", iconset, "-o", os.path.join(out, "icon.icns")],
        check=True,
    )
    print(f"wrote {out}/icon.icns")


if __name__ == "__main__":
    main()
