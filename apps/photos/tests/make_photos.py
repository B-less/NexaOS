"""Test pictures for Photos (scripts/test-photos-app.sh): JPEGs drawn with
ImageMagick, given real EXIF (date, camera, settings, place, orientation)
by hand, so the dates, months and details are known. Plus hostile files."""
import os
import struct
import subprocess
import sys


def tiff(date, orientation=1, make="NexaCam", model="Hex 1", gps=None):
    """Little-endian TIFF/EXIF: IFD0 (make, model, orientation, EXIF, GPS),
    the EXIF IFD (exposure, f-number, ISO, focal length, date), GPS."""
    e = lambda tag, kind, count, val: struct.pack("<HHII", tag, kind, count, val)
    make_b, model_b, date_b = make.encode() + b"\0", model.encode() + b"\0", date.encode() + b"\0"
    n0 = 5 if gps else 4
    ifd0 = 8
    data0 = ifd0 + 2 + n0 * 12 + 4
    sub = data0 + len(make_b) + len(model_b)
    rats = sub + 2 + 5 * 12 + 4
    gps_at = rats + 24 + len(date_b)
    b = b"II*\0" + struct.pack("<I", ifd0) + struct.pack("<H", n0)
    b += e(0x010F, 2, len(make_b), data0) + e(0x0110, 2, len(model_b), data0 + len(make_b))
    b += struct.pack("<HHIHH", 0x0112, 3, 1, orientation, 0) + e(0x8769, 4, 1, sub)
    if gps:
        b += e(0x8825, 4, 1, gps_at)
    b += struct.pack("<I", 0) + make_b + model_b
    b += struct.pack("<H", 5) + e(0x829A, 5, 1, rats) + e(0x829D, 5, 1, rats + 8)
    b += struct.pack("<HHIHH", 0x8827, 3, 1, 200, 0) + e(0x920A, 5, 1, rats + 16) + e(0x9003, 2, len(date_b), rats + 24)
    b += struct.pack("<I", 0) + struct.pack("<6I", 1, 250, 28, 10, 35, 1) + date_b
    if gps:
        (lat, ns), (lon, ew) = gps
        def dms(v):
            d = int(v); m = int((v - d) * 60); s = round(((v - d) * 60 - m) * 60 * 100)
            return struct.pack("<6I", d, 1, m, 1, s, 100)
        g = gps_at + 2 + 4 * 12 + 4
        b += struct.pack("<H", 4) + struct.pack("<HHI4s", 1, 2, 2, ns.encode() + b"\0\0\0") + e(2, 5, 3, g)
        b += struct.pack("<HHI4s", 3, 2, 2, ew.encode() + b"\0\0\0") + e(4, 5, 3, g + 24) + struct.pack("<I", 0)
        b += dms(lat) + dms(lon)
    return b


def with_exif(jpeg, exif):
    seg = b"Exif\0\0" + exif
    return jpeg[:2] + b"\xff\xe1" + struct.pack(">H", len(seg) + 2) + seg + jpeg[2:]


def draw(path, w, h, colours, shape):
    a, b = colours
    cmd = ["magick", "-size", f"{w}x{h}", f"gradient:{a}-{b}"]
    if shape == "sun":
        cmd += ["-fill", "#ffd27a", "-draw", f"circle {w * 0.7},{h * 0.32} {w * 0.7 + h * 0.1},{h * 0.32}",
                "-fill", "#1d3557", "-draw", f"polygon 0,{h} {w * 0.35},{h * 0.55} {w * 0.6},{h * 0.8} {w * 0.8},{h * 0.6} {w},{h}"]
    elif shape == "hex":
        cx, cy, r = w / 2, h / 2, h * 0.3
        pts = " ".join(f"{cx + r * c},{cy + r * s}" for c, s in ((0, -1), (0.866, -0.5), (0.866, 0.5), (0, 1), (-0.866, 0.5), (-0.866, -0.5)))
        cmd += ["-fill", "#ffffffaa", "-draw", f"polygon {pts}"]
    elif shape == "up":  # stored on its side: an arrow pointing right, which is up once turned upright (EXIF 8)
        cmd += ["-fill", "#ffffff", "-draw", f"polygon {w * 0.9},{h / 2} {w * 0.55},{h * 0.2} {w * 0.55},{h * 0.8}"]
    cmd += ["-quality", "88", "jpeg:-"]
    return subprocess.run(cmd, check=True, capture_output=True).stdout


PALETTES = [("#0f2027", "#2c5364"), ("#ff7e5f", "#feb47b"), ("#43cea2", "#185a9d"), ("#ee9ca7", "#ffdde1"),
            ("#141e30", "#243b55"), ("#f7971e", "#ffd200"), ("#8e2de2", "#4a00e0"), ("#56ab2f", "#a8e063")]
SHAPES = ["sun", "hex", "", "sun"]


def main(pics):
    os.makedirs(pics, exist_ok=True)
    # (folder, name, date, size, gps)
    plan = []
    for i in range(10):
        plan.append(("Holiday", f"beach-{i + 1:02d}.jpg", f"2026:10:{20 - i:02d} 1{i % 10}:0{i % 6}:00", (1200, 800),
                     ((5.6037, "N"), (0.187, "W")) if i == 0 else None))
    for i in range(8):
        plan.append(("Family", f"garden-{i + 1:02d}.jpg", f"2026:08:{12 + i:02d} 09:3{i % 6}:00", (900, 1200) if i % 3 == 0 else (1200, 900), None))
    for i in range(5):
        plan.append(("", f"old-{i + 1}.jpg", f"2025:03:0{i + 1} 12:00:00", (1000, 750), None))
    for n, (folder, name, date, (w, h), gps) in enumerate(plan):
        jpg = draw(None, w, h, PALETTES[n % len(PALETTES)], SHAPES[n % len(SHAPES)])
        d = os.path.join(pics, folder)
        os.makedirs(d, exist_ok=True)
        with open(os.path.join(d, name), "wb") as f:
            f.write(with_exif(jpg, tiff(date, gps=gps)))
    # Stored on its side: shown upright, the arrow pointing up.
    side = draw(None, 1200, 800, ("#283c86", "#45a247"), "up")
    with open(os.path.join(pics, "turned.jpg"), "wb") as f:
        f.write(with_exif(side, tiff("2026:10:25 08:00:00", orientation=8)))
    # Hostile or odd ones.
    open(os.path.join(pics, "empty.jpg"), "wb").close()
    with open(os.path.join(pics, "notes.jpg"), "w") as f:
        f.write("this is text, not a picture\n")
    png = subprocess.run(["magick", "-size", "640x480", "gradient:#ff512f-#dd2476", "png:-"], check=True, capture_output=True).stdout
    with open(os.path.join(pics, "really-a-png.jpg"), "wb") as f:
        f.write(png)
    quoted = draw(None, 800, 600, ("#1f4037", "#99f2c8"), "hex")
    with open(os.path.join(pics, "Holiday", 'it\'s "quoted" & spaced.jpg'), "wb") as f:
        f.write(with_exif(quoted, tiff("2026:10:21 18:00:00")))
    # A JPEG claiming to be 60000 x 60000 (its header lies).
    huge = bytearray(draw(None, 64, 48, ("#000000", "#ffffff"), ""))
    i = huge.find(b"\xff\xc0")
    huge[i + 5:i + 9] = struct.pack(">HH", 60000, 60000)
    with open(os.path.join(pics, "huge.jpg"), "wb") as f:
        f.write(with_exif(bytes(huge), tiff("2024:01:01 00:00:00")))
    cut = draw(None, 1200, 800, ("#373b44", "#4286f4"), "sun")
    with open(os.path.join(pics, "cut-short.jpg"), "wb") as f:
        f.write(with_exif(cut[:len(cut) // 3], tiff("2024:01:02 00:00:00")))
    # Hidden: never listed.
    os.makedirs(os.path.join(pics, ".thumbnails"), exist_ok=True)
    with open(os.path.join(pics, ".thumbnails", "x.jpg"), "wb") as f:
        f.write(with_exif(cut, tiff("2026:11:01 00:00:00")))


if __name__ == "__main__":
    main(sys.argv[1])
