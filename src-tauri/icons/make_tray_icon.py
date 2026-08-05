#!/usr/bin/env python3
"""Rasterize the PR Radar tray icon as a macOS template image.

Template images carry shape in the alpha channel only; macOS recolors them to
match the menu bar, so every pixel is black with a computed coverage alpha.
Anti-aliasing is 4x4 supersampled analytic coverage -- no image libraries here.
"""

import math
import struct
import zlib

# Geometry in unit coordinates, origin at the icon centre.
R_OUT, S_OUT = 0.435, 0.052          # outer ring radius / stroke
R_IN, S_IN = 0.205, 0.042            # inner ring radius / stroke
BEAM_ANGLE = 64.0                    # leading edge of the sweep, degrees CCW
WEDGE_FROM, WEDGE_TO = 6.0, 64.0     # the trailing sweep it drags behind it
WEDGE_ALPHA = 0.30
S_BEAM = 0.046                       # beam stroke
# Sits inside the swept arc but clear of the beam, so it reads as a contact
# rather than a bulge on the line.
BLIP_R, BLIP_AT, BLIP_ANGLE = 0.062, 0.325, 19.0
SS = 4                               # supersampling factor per axis


def coverage(x: float, y: float) -> float:
    """Alpha for a single sample point, in unit coordinates."""
    dx, dy = x - 0.5, 0.5 - y  # flip y so angles read counter-clockwise
    d = math.hypot(dx, dy)
    a = 0.0

    # Outer and inner rings.
    if abs(d - R_OUT) <= S_OUT / 2:
        a = 1.0
    if abs(d - R_IN) <= S_IN / 2:
        a = 1.0

    inner_edge = R_OUT - S_OUT / 2 - 0.012

    # Sweep beam: perpendicular distance to the ray leaving the centre.
    rad = math.radians(BEAM_ANGLE)
    ux, uy = math.cos(rad), math.sin(rad)
    along = dx * ux + dy * uy
    if 0.0 <= along <= inner_edge:
        if abs(-dx * uy + dy * ux) <= S_BEAM / 2:
            a = 1.0

    # Trailing wedge, drawn faint so the beam still reads as the leading edge.
    if d <= inner_edge and a < WEDGE_ALPHA:
        ang = math.degrees(math.atan2(dy, dx)) % 360.0
        if WEDGE_FROM <= ang <= WEDGE_TO:
            a = WEDGE_ALPHA

    # The contact.
    br = math.radians(BLIP_ANGLE)
    bx, by = BLIP_AT * math.cos(br), BLIP_AT * math.sin(br)
    if math.hypot(dx - bx, dy - by) <= BLIP_R:
        a = 1.0

    return a


def render(size: int, rgb=(0, 0, 0)) -> bytes:
    """RGBA pixel rows with supersampled coverage in the alpha channel.

    Tauri's `include_image!` only accepts RGBA, so this writes four channels
    rather than the more compact grayscale+alpha.
    """
    rows = []
    step = 1.0 / (size * SS)
    for py in range(size):
        row = bytearray()
        for px in range(size):
            total = 0.0
            for sy in range(SS):
                for sx in range(SS):
                    u = (px * SS + sx + 0.5) * step
                    v = (py * SS + sy + 0.5) * step
                    total += coverage(u, v)
            alpha = int(round(255 * total / (SS * SS)))
            row += bytes((rgb[0], rgb[1], rgb[2], alpha))
        rows.append(bytes(row))
    return rows


def write_png(path: str, size: int, rgb=(0, 0, 0)) -> None:
    rows = render(size, rgb)
    raw = b"".join(b"\x00" + r for r in rows)  # filter type 0 per scanline

    def chunk(tag: bytes, data: bytes) -> bytes:
        return (
            struct.pack(">I", len(data))
            + tag
            + data
            + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
        )

    png = b"\x89PNG\r\n\x1a\n"
    # bit depth 8, color type 6 = RGBA
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(raw, 9))
    png += chunk(b"IEND", b"")

    with open(path, "wb") as f:
        f.write(png)
    print(f"{path}  {size}x{size}  {len(png)} bytes")


if __name__ == "__main__":
    import sys

    out = sys.argv[1] if len(sys.argv) > 1 else "."

    # macOS template image: pure black, recolored by the system per appearance.
    # tray-icon scales any source to an 18pt height, so 36px is an exact 2x
    # match for Retina and downsamples cleanly 2:1 on a non-Retina display.
    write_png(f"{out}/tray.png", 36)
    write_png(f"{out}/tray@2x.png", 72)

    # Linux and Windows have no template-image concept, so the same black art
    # disappears against a dark panel. A near-white icon reads on the dark
    # panels that ship as the default nearly everywhere, and still has enough
    # contrast on light ones. 48px suits the larger tray sizes Linux uses.
    write_png(f"{out}/tray-color.png", 48, rgb=(0xE8, 0xED, 0xF5))
