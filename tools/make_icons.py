#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
AiClub 图标生成 —— 黑白体系,圆润,iOS 风。

身份符号来自 §10.6:一实心、一描边两枚气泡,左右错位对谈。
墨向极性是这个应用区分两个 AI 的唯一手段,所以图标就画这件事本身。

输出:src-tauri/icons/{32x32,128x128,128x128@2x,icon}.png, icon.ico, icon.icns,
      以及前端用的 appicon.png。超采样 4x 抗锯齿。
"""
import os
import struct
import subprocess
import sys

from PIL import Image, ImageDraw

SS = 4  # 超采样倍数
BASE = 1024
DARK = (26, 26, 31, 255)      # --sheet 曜灰
INK = (241, 242, 245, 255)    # --tx-1 / --ink
BG = (10, 10, 12, 255)        # --bg 曜灰(图标外围留一圈更深的底)
LINE = (241, 242, 245, 46)    # 发丝线


def canvas(size):
    im = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    return im, ImageDraw.Draw(im)


def rounded(d, box, r, fill=None, outline=None, width=1):
    d.rounded_rectangle(box, radius=r, fill=fill, outline=outline, width=width)


def bubble(d, box, r, tail, fill=None, outline=None, width=1):
    """气泡:圆角矩形 + 一枚朝外的三角尾巴。tail = 'l' | 'r'"""
    x0, y0, x1, y1 = box
    rounded(d, box, r, fill=fill, outline=outline, width=width)
    if tail == "l":
        tx, ty = x0 + r * 0.55, y1 - r * 0.2
        pts = [(tx - r * 0.55, ty + r * 1.15), (tx + r * 0.5, ty), (tx + r * 0.62, ty + r * 1.02)]
    else:
        tx, ty = x1 - r * 0.55, y1 - r * 0.2
        pts = [(tx + r * 0.55, ty + r * 1.15), (tx - r * 0.5, ty), (tx - r * 0.62, ty + r * 1.02)]
    d.polygon(pts, fill=fill if fill else (0, 0, 0, 0))
    if outline and not fill:
        # 空心气泡的尾巴也要描边:重画一次边线
        d.line(pts + [pts[0]], fill=outline, width=width, joint="curve")


def draw_master(size=BASE):
    s = size * SS
    im, d = canvas(s)
    u = s / 1024.0  # 单位

    # 底板:深石墨圆角方块,四角留透明(iOS 图标的圆角形状本身)
    pad = 40 * u
    rounded(d, [pad, pad, s - pad, s - pad], 200 * u, fill=DARK)
    # 一圈发丝线,让深色 dock 上也能读出色块边界
    rounded(d, [pad, pad, s - pad, s - pad], 200 * u, outline=LINE, width=int(3 * u))

    # 两枚气泡:左上实心(甲)、右下描边(乙),错位对谈
    bw, bh, br = 520 * u, 300 * u, 96 * u

    # 实心那枚靠上偏左
    box_a = [150 * u, 190 * u, 150 * u + bw, 190 * u + bh]
    bubble(d, box_a, br, "l", fill=INK)

    # 空心那枚靠下偏右
    box_b = [s - 150 * u - bw, s - 210 * u - bh, s - 150 * u, s - 210 * u]
    w = int(20 * u)
    bubble(d, box_b, br, "r", outline=INK, width=w)

    # 实心气泡里点三颗"空格"(反色),表示正在说话
    cy = box_a[1] + bh / 2
    for i in range(3):
        cx = box_a[0] + bw * (0.30 + i * 0.20)
        rr = 24 * u
        d.ellipse([cx - rr, cy - rr, cx + rr, cy + rr], fill=DARK)

    out = im.resize((size, size), Image.LANCZOS)
    return out


def write_ico(im, path):
    """手写 ICO(多个 PNG 压缩帧),避免依赖外部工具。"""
    sizes = [16, 24, 32, 48, 64, 128, 256]
    frames = []
    for sz in sizes:
        f = im.copy()
        f = f.resize((sz, sz), Image.LANCZOS)
        import io
        b = io.BytesIO()
        f.save(b, format="PNG")
        frames.append((sz, b.getvalue()))

    n = len(frames)
    hdr = struct.pack("<HHH", 0, 1, n)
    dir_ = b""
    data = b""
    offset = 6 + 16 * n
    for sz, blob in frames:
        w = 0 if sz >= 256 else sz
        h = 0 if sz >= 256 else sz
        dir_ += struct.pack("<BBBBHHII", w, h, 0, 0, 1, 32, len(blob), offset)
        offset += len(blob)
        data += blob
    with open(path, "wb") as fh:
        fh.write(hdr + dir_ + data)


def make_icns(im, outdir):
    """iconutil 优先(mac 上);本机没有就用 Pillow 能写的 fallback:
    直接把 png 集打包成 icns 不可行,故在 Windows 上生成一个仅含 1024 的 icns
    由 tauri 在 mac 构建时自行处理 —— 这里退化为写一份 icon_1024.png 并调用 iconutil(若存在)。"""
    sizes = [16, 32, 64, 128, 256, 512, 1024]
    tmp = os.path.join(outdir, "AiClub.iconset")
    os.makedirs(tmp, exist_ok=True)
    for s in sizes:
        im.resize((s, s), Image.LANCZOS).save(os.path.join(tmp, f"icon_{s}x{s}.png"))
    for s in [16, 32, 128, 256]:
        im.resize((s * 2, s * 2), Image.LANCZOS).save(os.path.join(tmp, f"icon_{s}x{s}@2x.png"))
    iconutil = "/usr/bin/iconutil"
    if os.path.exists(iconutil):
        target = os.path.join(outdir, "icon.icns")
        subprocess.run([iconutil, "-c", "icns", tmp, "-o", target], check=True)
        print(f"  icon.icns via iconutil")
    else:
        # Windows:用 pngify 方式写一个最小合法 icns(单 1024x1024 ic07 条目,PNG 载荷)
        big = im.resize((1024, 1024), Image.LANCZOS)
        import io
        b = io.BytesIO()
        big.save(b, format="PNG")
        png = b.getvalue()
        entries = [
            (b"ic10", 512, png),  # 512@2x
        ]
        s512 = io.BytesIO()
        im.resize((512, 512), Image.LANCZOS).save(s512, format="PNG")
        entries.append((b"ic07", 512, s512.getvalue()))
        body = b""
        for tag, _, blob in entries:
            body += tag + struct.pack(">I", len(blob) + 8) + blob
        head = b"icns" + struct.pack(">I", len(body) + 8) + body
        with open(os.path.join(outdir, "icon.icns"), "wb") as fh:
            fh.write(head)
        print("  icon.icns via fallback(PNG 条目)")
    import shutil
    shutil.rmtree(tmp, ignore_errors=True)


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    root = os.path.dirname(here)
    icons = os.path.join(root, "src-tauri", "icons")
    os.makedirs(icons, exist_ok=True)

    master = draw_master(BASE)
    master.save(os.path.join(icons, "icon.png"))
    for name, size in [("32x32.png", 32), ("64x64.png", 64), ("128x128.png", 128), ("256x256.png", 256), ("512x512.png", 512)]:
        master.resize((size, size), Image.LANCZOS).save(os.path.join(icons, name))
    master.resize((256, 256), Image.LANCZOS).save(os.path.join(icons, "128x128@2x.png"))

    write_ico(master, os.path.join(icons, "icon.ico"))
    make_icns(master, icons)

    # Windows Store 磁贴占位(不用 Store 分发,但 tauri 默认图标清单里会引用)
    for s in [44, 71, 89, 107, 142, 150, 284, 310]:
        master.resize((s, s), Image.LANCZOS).save(os.path.join(icons, f"Square{s}x{s}Logo.png"))
    master.resize((30, 30), Image.LANCZOS).save(os.path.join(icons, "Square30x30Logo.png"))

    dist = os.path.join(root, "src", "assets")
    os.makedirs(dist, exist_ok=True)
    master.resize((192, 192), Image.LANCZOS).save(os.path.join(dist, "appicon.png"))
    print("icons ->", icons)


if __name__ == "__main__":
    sys.exit(main())
