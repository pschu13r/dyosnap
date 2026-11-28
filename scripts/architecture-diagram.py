#!/usr/bin/env python3
"""Draws the C4 component diagram (level 3) of DyoSnap as docs/architecture.svg.

Edit the boxes and arrows below when the module layout changes, then run:
    python3 scripts/architecture-diagram.py
Needs only the Python standard library. Positions are pixels on a 2000x1420 canvas.
"""
from pathlib import Path
from xml.sax.saxutils import escape as e

OUT = Path(__file__).resolve().parent.parent / "docs" / "architecture.svg"
W, H = 2000, 1420
FONT = "Helvetica Neue, Helvetica, Arial, sans-serif"
P = []  # SVG elements

# (fill, stroke, text) in the usual C4 colours
COMP = ("#85BBF0", "#5D82A8", "#0B2545")
PLAT = ("#DCEBFA", "#5D82A8", "#0B2545")
EXT = ("#999999", "#6B6B6B", "#FFFFFF")
CONT = ("#438DD5", "#3C7FC0", "#FFFFFF")
PERSON = ("#08427B", "#073B6F", "#FFFFFF")


def box(x, y, w, h, name, tech, lines, style=COMP, align="middle", rx=10):
    fill, stroke, fg = style
    P.append(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{rx}" fill="{fill}" stroke="{stroke}" stroke-width="1.5"/>')
    tx, anchor = (x + w / 2, "middle") if align == "middle" else (x + 16, "start")
    cy = y + 28
    P.append(f'<text x="{tx}" y="{cy}" text-anchor="{anchor}" font-size="17" font-weight="700" fill="{fg}">{e(name)}</text>')
    cy += 20
    if tech:
        P.append(f'<text x="{tx}" y="{cy}" text-anchor="{anchor}" font-size="12.5" fill="{fg}" opacity="0.85">[{e(tech)}]</text>')
        cy += 24
    for ln in lines:
        P.append(f'<text x="{tx}" y="{cy}" text-anchor="{anchor}" font-size="13.5" fill="{fg}">{e(ln)}</text>')
        cy += 19


def line(points, label=None, lx=None, ly=None, anchor="middle", arrow=True, rotate=None):
    d = "M " + " L ".join(f"{px} {py}" for px, py in points)
    head = ' marker-end="url(#arr)"' if arrow else ""
    P.append(f'<path d="{d}" fill="none" stroke="#555" stroke-width="1.6"{head}/>')
    if label:
        rot = f' transform="rotate({rotate} {lx} {ly})"' if rotate else ""
        for i, t in enumerate(label.split("\n")):
            P.append(f'<text x="{lx}" y="{ly + i * 16}" text-anchor="{anchor}" font-size="12.5" fill="#333"{rot}>{e(t)}</text>')


# Title
P.append('<text x="560" y="62" font-size="28" font-weight="700" fill="#111">DyoSnap: component diagram (C4 level 3)</text>')
P.append('<text x="560" y="94" font-size="15" fill="#444">One code base and one UI (egui/eframe on OpenGL) on macOS and Linux. Only the platform layer differs per system.</text>')

# Person
P.append(f'<circle cx="370" cy="42" r="24" fill="{PERSON[0]}"/>')
box(260, 62, 220, 80, "User", "Person", [], style=PERSON, rx=26)

# Container boundary
P.append('<rect x="20" y="180" width="1640" height="1055" rx="6" fill="none" stroke="#777" stroke-width="1.6" stroke-dasharray="10 6"/>')
P.append('<text x="40" y="1205" font-size="17" font-weight="700" fill="#333">DyoSnap</text>')
P.append('<text x="40" y="1225" font-size="13" fill="#555">[Container: desktop app, one process. Rust, egui/eframe + OpenGL]</text>')

# Row 1
box(80, 220, 580, 125, "Main window & control", "Component: egui · app/ (mod, frame, window, events, views, theme), icons.rs",
    ["Main window, start page, bars. Handles the tray menu and",
     "global shortcuts, runs the screenshot and recording flow.",
     "Linux: app/linux.rs keeps the state while the window is gone."])
box(760, 220, 300, 125, "Settings window", "Component: egui · prefs.rs, about.rs",
    ["Own window with General,", "Advanced, Hotkeys and About tabs."])
box(1160, 220, 300, 125, "Settings", "Component: serde_json · settings.rs",
    ["User settings in a small JSON file,", "read by almost every component."])
line([(660, 282), (758, 282)], "opens", 709, 272)
line([(1060, 282), (1158, 282)], "changes", 1109, 272)

line([(370, 142), (370, 218)], "uses tray, shortcuts, windows", 382, 166, anchor="start")

# Row 2
xs = [80, 395, 710, 1025, 1340]
w2 = 285
c = [x + w2 / 2 for x in xs]
box(xs[0], 435, w2, 130, "Capture flow", "Component: egui · app/capture/",
    ["Screenshot jobs, choosing the", "record area, overlays and the", "recording bar (recbar.rs)."])
box(xs[1], 435, w2, 130, "Files", "Component: Rust · app/files.rs",
    ["Opens, saves and copies", "pictures and videos (local,", "no network)."])
box(xs[2], 435, w2, 130, "Image editor", "Component: egui · editor.rs",
    ["Canvas: draw, select, move,", "type text, crop."])
box(xs[3], 435, w2, 130, "Video trimmer", "Component: egui · video.rs",
    ["Preview after recording,", "pick start and end on the", "timeline, then save."])
box(xs[4], 435, w2, 130, "Screen recording", "Component: Rust · record.rs",
    ["Starts and stops recording in", "a separate process. The app", "only shows a timer."])

# Bus from the main window to row 2
line([(370, 345), (370, 395)], arrow=False)
line([(c[0], 395), (c[4], 395)], arrow=False)
P.append('<text x="382" y="378" font-size="12.5" fill="#333">controls</text>')
for cx in c:
    line([(cx, 395), (cx, 433)])

# Row 3
box(470, 660, 480, 120, "Image document", "Component: tiny-skia, ab_glyph · doc.rs",
    ["Base image plus a list of editable shapes. Renders the",
     "final picture with tiny-skia, only when it changes."])
line([(c[1], 565), (c[1], 658)], "loads, exports", c[1] + 10, 615, anchor="start")
line([(c[2], 565), (c[2], 658)], "edits shapes", c[2] + 10, 615, anchor="start")

# Platform layer
P.append('<rect x="80" y="850" width="1545" height="320" rx="10" fill="#C9DEF5" stroke="#5D82A8" stroke-width="1.5"/>')
P.append('<text x="100" y="878" font-size="17" font-weight="700" fill="#0B2545">Platform layer</text>')
P.append('<text x="232" y="878" font-size="12.5" fill="#0B2545" opacity="0.85">[Component: Rust · platform/mod.rs]</text>')
P.append('<text x="100" y="900" font-size="13.5" fill="#0B2545">The same functions on both systems: screenshots, clipboard, screens, tray, shortcuts, recording, opening files, notifications, launch at login. Picked with cfg(target_os) at compile time.</text>')

mac = ["Tray & shortcuts: tray-icon, global-hotkey",
       "Screenshots: screencapture (child process)",
       "Area picker: in-process picture with ScreenCaptureKit",
       "Clipboard: NSPasteboard (objc2)",
       "Screens, mouse position: CoreGraphics",
       "Recording: ffmpeg with AVFoundation, else screencapture -v",
       "Notifications: UserNotifications · Open files: open",
       "Permissions: reads the Privacy & Security state, no prompt"]
lin = ["Tray: ksni (StatusNotifierItem over D-Bus)",
       "Shortcuts, screencast: xdg-desktop-portal (ashpd)",
       "Screenshots: Screenshot portal · Area picker: frozen PipeWire picture",
       "Recording: PipeWire stream, fed to ffmpeg through a named pipe",
       "Record frame: wlr-layer-shell (not GNOME) · Raise: xdg-activation",
       "Clipboard: arboard, on GNOME plain Wayland (wl_data_device)",
       "Screens: Wayland (smithay-client-toolkit) · Windows: KWin script",
       "Window frame: winit Adwaita CSD · Open files: xdg-open, gio",
       "One instance: org.freedesktop.Application on the session bus"]
box(100, 915, 745, 240, "macOS implementation", "Component: Rust · platform/macos/", mac, style=PLAT, align="start")
box(860, 915, 745, 240, "Linux implementation (Wayland only)", "Component: Rust · platform/linux/", lin, style=PLAT, align="start")

line([(c[0], 565), (c[0], 848)], "screenshots, screens", c[0] + 10, 715, anchor="start")
line([(c[4], 565), (c[4], 848)], "screen source,\nffmpeg arguments", c[4] + 10, 715, anchor="start")
line([(80, 300), (48, 300), (48, 980), (78, 980)], "tray, shortcuts, clipboard, permissions", 38, 640, rotate=-90)

# ffmpeg
box(1720, 435, 260, 130, "ffmpeg", "Container: child process",
    ["Records, encodes, decodes.", "macOS: bundled (LGPL)", "Linux: from the system"], style=CONT)
line([(xs[4] + w2, 500), (1718, 500)], "starts", 1667, 490, anchor="start")
line([(c[3], 565), (c[3], 615), (1800, 615), (1800, 567)], "starts for preview frames and export", 1320, 607)
line([(1605, 1000), (1930, 1000), (1930, 567)], "raw video over a named pipe", 1767, 990)

# File system
box(1720, 220, 260, 125, "File system", "external",
    ["settings.json, pictures, videos"], style=EXT)
line([(1460, 282), (1718, 282)], "reads, writes settings.json", 1560, 272)

# Operating systems
box(270, 1285, 400, 125, "macOS", "Software system",
    ["AppKit, CoreGraphics, ScreenCaptureKit, AVFoundation,", "UserNotifications, screencapture,", "Screen Recording permission"], style=EXT)
box(1032, 1285, 400, 125, "Linux desktop (GNOME, KDE)", "Software system",
    ["Wayland compositor, xdg-desktop-portal,", "PipeWire, D-Bus"], style=EXT)
line([(470, 1155), (470, 1283)], "system calls, child processes", 482, 1260, anchor="start")
line([(1232, 1155), (1232, 1283)], "D-Bus, Wayland, PipeWire", 1244, 1260, anchor="start")

# Legend
lx, ly = 1720, 1250
P.append(f'<text x="{lx}" y="{ly}" font-size="14" font-weight="700" fill="#333">Legend</text>')
items = [(PERSON[0], "Person"), (COMP[0], "Component"), (PLAT[0], "Component, one per system"),
         (CONT[0], "External container"), (EXT[0], "External system")]
for i, (col, t) in enumerate(items):
    yy = ly + 14 + i * 26
    P.append(f'<rect x="{lx}" y="{yy}" width="26" height="18" rx="4" fill="{col}" stroke="#666" stroke-width="0.8"/>')
    P.append(f'<text x="{lx + 36}" y="{yy + 14}" font-size="13" fill="#333">{t}</text>')

svg = f'''<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}" font-family="{FONT}">
<defs><marker id="arr" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="8" markerHeight="8" orient="auto-start-reverse"><path d="M0 0 L10 5 L0 10 z" fill="#555"/></marker></defs>
<rect width="{W}" height="{H}" fill="#FFFFFF"/>
{chr(10).join(P)}
</svg>
'''
OUT.parent.mkdir(exist_ok=True)
OUT.write_text(svg, encoding="utf-8")
print(f"wrote {OUT.name}")
