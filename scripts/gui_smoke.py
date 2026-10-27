"""Linux startup smoke test with an isolated repository/config and private Xvfb.

Usage: python3 scripts/gui_smoke.py /path/to/recorde /path/to/artifacts [--siege] [--light] [--dpi=1.5] [--review]
Run through scripts/linux_task.py for bounded ownership and descendant cleanup.
--review requires RECORDE_UI_INSPECTOR and an app built with ui-inspection; it
adds native Help, column preferences and repository-browser acceptance captures.
Requires Xvfb. Image mode also requires libXtst and selects a working image,
verifying both snapshots' colors in the framebuffer. Line mode checks selected
index bytes and worktree preservation. Other mode checks startup.
"""
import os
import json
from pathlib import Path
import socket
import struct
import subprocess
import sys
import tempfile
import time
import zlib


def png_from_xwd(source, destination):
    raw = source.read_bytes()
    header = struct.unpack(">25I", raw[:100])
    size, width, height, bpp, stride, colors = (header[i] for i in (0, 4, 5, 11, 12, 19))
    assert bpp == 32 and header[7] == 0, "Expected little-endian 32-bit framebuffer"
    pixels = raw[size + colors * 12:]
    rows = []
    for y in range(height):
        row = pixels[y * stride:y * stride + width * 4]
        rgb = bytearray()
        for x in range(width):
            b, g, r, _ = row[x * 4:x * 4 + 4]
            rgb.extend((r, g, b))
        rows.append(b"\0" + rgb)

    def chunk(tag, data):
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    destination.write_bytes(b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">2I5B", width, height, 8, 2, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(b"".join(rows))) + chunk(b"IEND", b""))


def main():
    binary, artifacts = Path(sys.argv[1]).resolve(), Path(sys.argv[2]).resolve()
    recent_menu = "--recent-menu" in sys.argv[3:]
    assert not recent_menu or "--design-review" in sys.argv[3:], "Recent menu fixture requires --design-review"
    feature_review = "--review" in sys.argv[3:]
    design_review = "--design-review" in sys.argv[3:] or feature_review
    ui_scale = 1.25 if "--scaled" in sys.argv[3:] else 1.0
    native_scale = float(next((arg.split("=", 1)[1] for arg in sys.argv[3:] if arg.startswith("--dpi=")), "1"))
    assert native_scale in (1.0, 1.25, 1.5, 2.0), "Supported native DPI: 1, 1.25, 1.5, 2"
    inspector = os.environ.get("RECORDE_UI_INSPECTOR")
    assert not feature_review or inspector, "--review requires RECORDE_UI_INSPECTOR pointing to the native inspect_ui example"
    inspection_address = None
    if inspector:
        with socket.socket() as probe:
            probe.bind(("127.0.0.1", 0))
            inspection_address = f"127.0.0.1:{probe.getsockname()[1]}"
    compact = "--compact" in sys.argv[3:]
    welcome = "--welcome" in sys.argv[3:]
    light = "--light" in sys.argv[3:]
    palette = "siege" if "--siege" in sys.argv[3:] else "classic"
    image_preview = "--image-preview" in sys.argv[3:]
    split_selection = "--split-selection" in sys.argv[3:]
    line_selection = "--line-selection" in sys.argv[3:] or split_selection
    frame_profile = "--frame-profile" in sys.argv[3:]
    repository_settings = "--repository-settings" in sys.argv[3:]
    assert not (image_preview and line_selection), "Select one GUI fixture"
    assert not feature_review or not (welcome or image_preview or line_selection or recent_menu or repository_settings or frame_profile), "--review requires the standard design fixture without welcome or another action fixture"
    artifacts.mkdir(parents=True, exist_ok=True)
    (artifacts / "smoke-settings.json").write_text(json.dumps({
        "palette": palette, "dark_theme": not light, "native_scale": native_scale,
        "ui_scale": ui_scale, "feature_review": feature_review,
        "inspection": bool(inspector), "binary": str(binary),
    }, indent=2))
    with tempfile.TemporaryDirectory(prefix="recorde-gui-") as temporary:
        root = Path(temporary)
        repo = root / "fixture"
        repo.mkdir()
        (root / "runtime").mkdir(mode=0o700)
        env = os.environ.copy()
        env.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=str(root / "gitconfig"),
            XDG_CONFIG_HOME=str(root / "config"), XDG_DATA_HOME=str(root / "data"),
            XDG_RUNTIME_DIR=str(root / "runtime"), RUST_LOG="warn")
        env["WINIT_X11_SCALE_FACTOR"] = str(native_scale)
        env.pop("EGUI_INSPECTION_ADDR", None)
        if inspection_address:
            env["EGUI_INSPECTION"] = inspection_address
        else:
            env.pop("EGUI_INSPECTION", None)
        env.pop("WAYLAND_DISPLAY", None)
        env.pop("WAYLAND_SOCKET", None)
        if frame_profile:
            env["RECORDE_PROFILE_FRAMES"] = "1"

        def git(*args):
            subprocess.run(["git", "-C", str(repo), *args], env=env, check=True, stdout=subprocess.DEVNULL)

        git("init", "-b", "main")
        git("config", "user.name", "GUI Fixture")
        git("config", "user.email", "gui@example.invalid")
        def solid_png(color):
            def chunk(tag, data):
                return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
            return (b"\x89PNG\r\n\x1a\n"
                + chunk(b"IHDR", struct.pack(">2I5B", 64, 64, 8, 2, 0, 0, 0))
                + chunk(b"IDAT", zlib.compress((b"\0" + bytes(color) * 64) * 64))
                + chunk(b"IEND", b""))

        if image_preview:
            (repo / "image.png").write_bytes(solid_png((255, 0, 0)))
        else:
            (repo / "file.txt").write_text("base\n")
        git("add", ".")
        git("commit", "-m", "Initial fixture")
        if frame_profile:
            parent = subprocess.check_output(["git", "-C", str(repo), "rev-parse", "HEAD"], env=env).strip()
            record = b"commit refs/heads/main\ncommitter GUI Fixture <gui@example.invalid> 1700000000 +0000\ndata 7\nhistory\n"
            history = record + b"from " + parent + b"\n\n" + (record + b"\n") * 99_999 + b"done\n"
            subprocess.run(["git", "-C", str(repo), "fast-import", "--quiet"], input=history, env=env, check=True)
        if image_preview:
            (repo / "image.png").write_bytes(solid_png((0, 0, 255)))
        else:
            (repo / "file.txt").write_text("base\nfirst\nmiddle\nlast\n" if line_selection else "external working change\n")
        if design_review:
            git("checkout", "-b", "feature/workspace")
            (repo / "workspace.rs").write_text('pub fn title() -> str { "Your workspace" }\n')
            git("add", "workspace.rs")
            git("commit", "-m", "Add workspace navigation")
            git("checkout", "main")
            (repo / "theme.rs").write_text('// Theme tokens\n')
            git("add", "theme.rs")
            git("commit", "-m", "Introduce adaptive theme tokens")
            git("merge", "--no-ff", "feature/workspace", "-m", "Merge workspace navigation")
            git("checkout", "-b", "feature/visual-refresh")
            for name, message in [("icons.rs", "Bundle icons for every platform"), ("layout.rs", "Make room for the commit graph"), ("search.rs", "Improve repository search")]:
                (repo / name).write_text('// Implementation\n')
                git("add", name)
                git("commit", "-m", message)
            (repo / "theme.rs").write_text('// Theme tokens\n// Add warm light surfaces\n')
            git("add", "theme.rs")
            (repo / "workspace.rs").write_text('pub fn title() -> str { "Welcome to Recorde" }\n')
            (repo / "accessibility-notes-with-a-long-filename.md").write_text('Keyboard and icon review\n')
        settings = root / "config" / "recorde"
        settings.mkdir(parents=True, exist_ok=True)
        (settings / "config.toml").write_text(f'dark_theme = {str(not light).lower()}\ntheme_palette = "{palette}"\nfont_scale = {ui_scale}\n')
        if recent_menu:
            recent_paths = [repo, root / 'projects' / 'recorde', root / 'archive' / 'recorde', root / 'team' / 'a-repository-with-a-very-long-name', root / 'labs' / 'café-client']
            recent_paths += [root / 'projects' / name for name in ['website', 'shared-components', 'documentation']]
            for recent_path in recent_paths[1:]:
                recent_path.mkdir(parents=True)
                subprocess.run(['git', 'init', '-b', 'main', str(recent_path)], env=env, check=True, stdout=subprocess.DEVNULL)
            with (settings / 'config.toml').open('a') as config:
                config.write('recent_repos = ' + json.dumps([str(p) for p in recent_paths], ensure_ascii=False) + '\n')
        # TCP avoids WSLg's read-only /tmp/.X11-unix; an ephemeral cookie protects it.
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            display = sock.getsockname()[1] - 6000
        assert display > 0
        auth = root / "Xauthority"
        fields = [b"", str(display).encode(), b"MIT-MAGIC-COOKIE-1", os.urandom(16)]
        auth.write_bytes(struct.pack(">H", 65535) + b"".join(struct.pack(">H", len(f)) + f for f in fields))
        auth.chmod(0o600)
        env.update(DISPLAY=f"127.0.0.1:{display}", XAUTHORITY=str(auth))
        width, height = ((1100, 760) if compact else (1440, 900)) if design_review else (1280, 800)
        width, height = int(width * native_scale), int(height * native_scale)
        with (artifacts / "xvfb.log").open("w") as server_log:
            server = subprocess.Popen(["Xvfb", f":{display}", "-screen", "0", f"{width}x{height}x24",
                "-nolisten", "unix", "-listen", "tcp", "-auth", str(auth), "-fbdir", temporary],
                stdout=server_log, stderr=server_log)
            try:
                deadline = time.monotonic() + 5
                while not (root / "Xvfb_screen0").exists():
                    if server.poll() is not None or time.monotonic() > deadline:
                        raise RuntimeError("Xvfb did not start; see xvfb.log")
                    time.sleep(0.05)
                with (artifacts / "gui-smoke.log").open("w") as log:
                    app = subprocess.Popen([str(binary), *([] if welcome else [str(repo)])], env=env, stdout=log, stderr=log)
                    try:
                        # Font/GL initialization varies on shared CI runners. Wait
                        # for the actual window, with a deadline, before resizing.
                        resize = '''import ctypes
import time
x = ctypes.CDLL("libX11.so.6")
x.XOpenDisplay.argtypes = [ctypes.c_char_p]
x.XOpenDisplay.restype = ctypes.c_void_p
x.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
x.XDefaultRootWindow.restype = ctypes.c_ulong
x.XQueryTree.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.POINTER(ctypes.c_ulong)), ctypes.POINTER(ctypes.c_uint)]
x.XFetchName.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.POINTER(ctypes.c_char_p)]
x.XResizeWindow.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_uint, ctypes.c_uint]
x.XSetInputFocus.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_ulong]
class Attributes(ctypes.Structure):
    _fields_ = [(name, ctypes.c_int) for name in ('x', 'y', 'width', 'height', 'border_width', 'depth')] + [('visual', ctypes.c_void_p), ('root', ctypes.c_ulong)] + [(name, ctypes.c_int) for name in ('window_class', 'bit_gravity', 'win_gravity', 'backing_store')] + [('backing_planes', ctypes.c_ulong), ('backing_pixel', ctypes.c_ulong), ('save_under', ctypes.c_int), ('colormap', ctypes.c_ulong), ('map_installed', ctypes.c_int), ('map_state', ctypes.c_int)] + [(name, ctypes.c_long) for name in ('all_event_masks', 'your_event_mask', 'do_not_propagate_mask')] + [('override_redirect', ctypes.c_int), ('screen', ctypes.c_void_p)]
x.XGetWindowAttributes.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.POINTER(Attributes)]
x.XFlush.argtypes = [ctypes.c_void_p]
x.XCloseDisplay.argtypes = [ctypes.c_void_p]
x.XFree.argtypes = [ctypes.c_void_p]
d = x.XOpenDisplay(None)
assert d
deadline = time.monotonic() + 20
while True:
    root, parent, children, count = ctypes.c_ulong(), ctypes.c_ulong(), ctypes.POINTER(ctypes.c_ulong)(), ctypes.c_uint()
    x.XQueryTree(d, x.XDefaultRootWindow(d), ctypes.byref(root), ctypes.byref(parent), ctypes.byref(children), ctypes.byref(count))
    found = False
    for i in range(count.value):
        name = ctypes.c_char_p()
        x.XFetchName(d, children[i], ctypes.byref(name))
        if name.value and b'Recorde' in name.value:
            attributes = Attributes()
            x.XGetWindowAttributes(d, children[i], ctypes.byref(attributes))
            if attributes.map_state == 2:  # IsViewable; a created window may not yet be mapped.
                x.XResizeWindow(d, children[i], WIDTH, HEIGHT)
                x.XSetInputFocus(d, children[i], 1, 0)
                found = True
        if name: x.XFree(name)
    if children: x.XFree(children)
    if found:
        break
    assert time.monotonic() < deadline, 'Recorde native window not found within 20 seconds; see gui-smoke.log'
    time.sleep(0.1)
x.XFlush(d)
x.XCloseDisplay(d)
'''
                        subprocess.run([sys.executable, "-c", resize.replace("WIDTH", str(width)).replace("HEIGHT", str(height))], env=env, check=True)
                        time.sleep(5)
                        if app.poll() is not None:
                            raise RuntimeError(f"GUI exited with code {app.returncode}; see gui-smoke.log")
                        if image_preview or line_selection or repository_settings or (design_review and not welcome):
                            # Select the sole working file in the fixed 1280x800 fixture.
                            # Use a child so Xlib reads only this private display/cookie.
                            click = '''import ctypes
x = ctypes.CDLL("libX11.so.6")
t = ctypes.CDLL("libXtst.so.6")
x.XOpenDisplay.argtypes = [ctypes.c_char_p]
x.XOpenDisplay.restype = ctypes.c_void_p
x.XFlush.argtypes = [ctypes.c_void_p]
x.XCloseDisplay.argtypes = [ctypes.c_void_p]
t.XTestFakeMotionEvent.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_ulong]
t.XTestFakeButtonEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]
d = x.XOpenDisplay(None)
assert d
t.XTestFakeMotionEvent(d, -1, 1090, 258, 0)
t.XTestFakeButtonEvent(d, 1, 1, 0)
t.XTestFakeButtonEvent(d, 1, 0, 20)
x.XFlush(d)
x.XCloseDisplay(d)
'''
                            if design_review:
                                click = click.replace("1090, 258", f"{int(width - 190 * ui_scale * native_scale)}, {int(294 * ui_scale * native_scale)}")

                            subprocess.run([sys.executable, "-c", click], env=env, check=True)
                            time.sleep(2)
                        if '--hover-review' in sys.argv[3:]:
                            assert design_review and not welcome and ui_scale == 1.0
                            for label, px, py in [('toolbar-hover', 235, 115), ('graph-hover', 300, 173), ('stage-hover', width - 46, 218), ('icon-hover', width - 36, 115)]:
                                motion = click.split('t.XTestFakeMotionEvent(d, -1,')[0]
                                motion += f't.XTestFakeMotionEvent(d, -1, {px}, {py}, 0)\nx.XFlush(d)\nx.XCloseDisplay(d)'
                                subprocess.run([sys.executable, '-c', motion], env=env, check=True)
                                time.sleep(0.4)
                                png_from_xwd(root / 'Xvfb_screen0', artifacts / f'{label}.png')
                        if recent_menu:
                            # Fixed native fixture: open the toolbar's repository chevron.
                            # Verify removal only changes history, then open another real repo.
                            base_click = click.replace(f"{int(width - 190 * ui_scale * native_scale)}, {int(294 * ui_scale * native_scale)}", "1090, 258")
                            def menu_click(px, py):
                                subprocess.run([sys.executable, "-c", base_click.replace("1090, 258", f"{px}, {py}")], env=env, check=True)
                                time.sleep(0.7)
                            menu_click(132, 115)
                            png_from_xwd(root / 'Xvfb_screen0', artifacts / 'recent-menu.png')
                            if '--recent-actions' in sys.argv[3:]:
                                menu_click(496, 271)
                                time.sleep(1)
                                import tomllib
                                stored = tomllib.loads((settings / 'config.toml').read_text())
                                assert str(recent_paths[1]) not in stored['recent_repos'], 'Remove action did not update recent history'
                                assert (recent_paths[1] / '.git').is_dir(), 'Remove action deleted repository files'
                                menu_click(132, 115)
                                menu_click(275, 271)
                                time.sleep(3)
                                stored = tomllib.loads((settings / 'config.toml').read_text())
                                assert stored.get('active_repo') == str(recent_paths[2]), 'Recent row did not open the selected repository'
                                assert stored['recent_repos'][0] == str(recent_paths[2]), 'Opening a recent row did not promote it'
                                print('Recent menu: remove preserves repository, open updates active repo and history')
                        if repository_settings:
                            menu = click.replace("1090, 258", "1238, 114")
                            subprocess.run([sys.executable, "-c", menu], env=env, check=True)
                            time.sleep(1)
                            png_from_xwd(root / "Xvfb_screen0", artifacts / "settings-menu.png")
                            choose = click.replace("1090, 258", "600, 160")
                            subprocess.run([sys.executable, "-c", choose], env=env, check=True)
                            time.sleep(2)
                            png_from_xwd(root / "Xvfb_screen0", artifacts / "settings-top.png")
                            scroll = click.split("t.XTestFakeMotionEvent(d, -1, 1090, 258, 0)")[0] + '''import time
t.XTestFakeMotionEvent(d, -1, 700, 460, 0)
for _ in range(9):
    t.XTestFakeButtonEvent(d, 5, 1, 0)
    t.XTestFakeButtonEvent(d, 5, 0, 20)
    x.XFlush(d)
    time.sleep(0.1)
x.XCloseDisplay(d)'''
                            subprocess.run([sys.executable, "-c", scroll], env=env, check=True)
                            time.sleep(2)
                        if line_selection:
                            select = click.split("t.XTestFakeMotionEvent(d, -1, 1090, 258, 0)")[0] + '''import time
t.XTestFakeKeyEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]
x.XKeysymToKeycode.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
x.XKeysymToKeycode.restype = ctypes.c_uint
# Xvfb has no window manager to assign keyboard focus on mouse clicks.
x.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
x.XDefaultRootWindow.restype = ctypes.c_ulong
x.XQueryPointer.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_uint)]
x.XSetInputFocus.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_ulong]
root_ret, child = ctypes.c_ulong(), ctypes.c_ulong()
a, b, c, e, mask = ctypes.c_int(), ctypes.c_int(), ctypes.c_int(), ctypes.c_int(), ctypes.c_uint()
x.XQueryPointer(d, x.XDefaultRootWindow(d), ctypes.byref(root_ret), ctypes.byref(child), ctypes.byref(a), ctypes.byref(b), ctypes.byref(c), ctypes.byref(e), ctypes.byref(mask))
assert child.value, "App window not under pointer"
x.XSetInputFocus(d, child, 1, 0)
x.XFlush(d)
time.sleep(0.3)
def press(px, py):
    t.XTestFakeMotionEvent(d, -1, px, py, 0)
    t.XTestFakeButtonEvent(d, 1, 1, 0)
    t.XTestFakeButtonEvent(d, 1, 0, 20)
    x.XFlush(d)
    time.sleep(0.3)
press(600, 642)
control = x.XKeysymToKeycode(d, 0xffe3)
t.XTestFakeKeyEvent(d, control, 1, 0)
press(600, 686)
t.XTestFakeKeyEvent(d, control, 0, 0)
press(250, 568)
x.XCloseDisplay(d)'''
                            if split_selection:
                                subprocess.run([sys.executable, "-c", click.replace("1090, 258", "858, 490")], env=env, check=True)
                                time.sleep(1)
                                png_from_xwd(root / "Xvfb_screen0", artifacts / "split-menu.png")
                                select = select.replace("press(600, 642)", "press(910, 590)\npress(700, 642)").replace("press(600, 686)", "press(700, 686)")
                            subprocess.run([sys.executable, "-c", select], env=env, check=True)
                            time.sleep(2)
                            png_from_xwd(root / "Xvfb_screen0", artifacts / "gui-smoke.png")
                            if split_selection:
                                assert "split = true" in (settings / "config.toml").read_text(), "Split mode was not activated"
                            indexed = subprocess.check_output(["git", "-C", str(repo), "show", ":file.txt"], env=env)
                            assert indexed == b"base\nfirst\nlast\n", f"Incorrect selected index content: {indexed!r}"
                            assert (repo / "file.txt").read_bytes() == b"base\nfirst\nmiddle\nlast\n"
                        if image_preview:
                            png_from_xwd(root / "Xvfb_screen0", artifacts / "gui-smoke.png")
                            raw = (root / "Xvfb_screen0").read_bytes()
                            header = struct.unpack(">25I", raw[:100])
                            pixels = raw[header[0] + header[19] * 12:]
                            # Red before / blue after must both reach the rendered diff area.
                            bottom = pixels[550 * header[12]:772 * header[12]]
                            colors = [bottom[i:i+3] for i in range(0, len(bottom), 4)]
                            assert colors.count(bytes((0, 0, 255))) > 100, "Before image was not rendered"
                            assert colors.count(bytes((255, 0, 0))) > 100, "After image was not rendered"
                        png_from_xwd(root / "Xvfb_screen0", artifacts / "gui-smoke.png")
                        if inspector:
                            inspector_args = [inspector, inspection_address, str(artifacts)]
                            if design_review and not welcome:
                                inspector_args.append("--graph")
                            if feature_review:
                                inspector_args.append("--review")
                            with (artifacts / "inspector.stdout.log").open("w") as inspector_out, (artifacts / "inspector.stderr.log").open("w") as inspector_err:
                                subprocess.run(inspector_args, env=env, check=True, timeout=120, stdout=inspector_out, stderr=inspector_err)
                            display_info = json.loads((artifacts / "display.json").read_text())
                            assert abs(display_info["pixels_per_point"] - native_scale * ui_scale) < 0.01, display_info
                            capture = (artifacts / "native-frame.png").read_bytes()
                            assert struct.unpack(">II", capture[16:24]) == (width, height), "Native screenshot has wrong DPI dimensions"
                        if frame_profile:
                            deadline = time.monotonic() + 20
                            while "RECORDE_FRAME_METRICS" not in (artifacts / "gui-smoke.log").read_text(errors="replace"):
                                assert app.poll() is None and time.monotonic() < deadline, "Native frame profile did not finish"
                                time.sleep(0.2)
                            for line in (artifacts / "gui-smoke.log").read_text(errors="replace").splitlines():
                                if "RECORDE_FRAME_METRICS" in line:
                                    print(line)
                        print("GUI startup and image comparison passed; framebuffer captured" if image_preview
                            else "GUI line selection staged only the selected lines; framebuffer captured" if line_selection
                            else "GUI window became ready and remained running; framebuffer captured")
                    finally:
                        if app.poll() is None:
                            app.terminate()
                            try:
                                app.wait(timeout=5)
                            except subprocess.TimeoutExpired:
                                app.kill()
                                app.wait(timeout=5)
            finally:
                if server.poll() is None:
                    server.terminate()
                    try:
                        server.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        server.kill()
                        server.wait(timeout=5)


if __name__ == "__main__":
    main()
