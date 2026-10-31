# The disk image's window, for dmgbuild (scripts/bundle.sh runs it). dmgbuild writes Finder's
# layout file itself, so building the image never opens a Finder window.
#
# `stage` is the folder holding Akyba.app and "How to open Akyba.txt"; the Applications shortcut
# is added here. The background (design/dmg-background.html) frames each item, so the positions
# below are the frames' centres.
import os.path

stage = defines["stage"]  # noqa: F821, set by dmgbuild from `-D stage=…`

# LZMA: tighter than the default zlib, and opened by every macOS Akyba runs on.
format = "ULMO"
files = [os.path.join(stage, "Akyba.app"), os.path.join(stage, "How to open Akyba.txt")]
symlinks = {"Applications": "/Applications"}

# The disk itself wears Akyba's icon on the desktop and in Finder's sidebar.
icon = "crates/akyba/assets/app/Akyba.icns"
background = "crates/akyba/assets/app/dmg-background.tiff"

window_rect = ((200, 120), (640, 440))
default_view = "icon-view"
show_status_bar = False
show_tab_view = False
show_toolbar = False
show_pathbar = False
show_sidebar = False
icon_size = 96
text_size = 12
icon_locations = {
    "Akyba.app": (170, 140),
    "Applications": (470, 140),
    "How to open Akyba.txt": (320, 342),
}
