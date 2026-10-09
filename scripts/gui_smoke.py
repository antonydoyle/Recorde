"""Linux startup smoke test with an isolated repository/config and private Xvfb.

Usage: python3 scripts/gui_smoke.py /path/to/recorde /path/to/artifacts [--image-preview | --line-selection]
Requires Xvfb. Image mode also requires libXtst and selects a working image,
verifying both snapshots' colors in the framebuffer. Line mode checks selected
index bytes and worktree preservation. Other mode checks startup.
"""
import os
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
    image_preview = "--image-preview" in sys.argv[3:]
    split_selection = "--split-selection" in sys.argv[3:]
    line_selection = "--line-selection" in sys.argv[3:] or split_selection
    frame_profile = "--frame-profile" in sys.argv[3:]
    assert not (image_preview and line_selection), "Select one GUI fixture"
    artifacts.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="recorde-gui-") as temporary:
        root = Path(temporary)
        repo = root / "fixture"
        repo.mkdir()
        (root / "runtime").mkdir(mode=0o700)
        env = os.environ.copy()
        env.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=str(root / "gitconfig"),
            XDG_CONFIG_HOME=str(root / "config"), XDG_DATA_HOME=str(root / "data"),
            XDG_RUNTIME_DIR=str(root / "runtime"), RUST_LOG="warn")
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
        with (artifacts / "xvfb.log").open("w") as server_log:
            server = subprocess.Popen(["Xvfb", f":{display}", "-screen", "0", "1280x800x24",
                "-nolisten", "unix", "-listen", "tcp", "-auth", str(auth), "-fbdir", temporary],
                stdout=server_log, stderr=server_log)
            try:
                deadline = time.monotonic() + 5
                while not (root / "Xvfb_screen0").exists():
                    if server.poll() is not None or time.monotonic() > deadline:
                        raise RuntimeError("Xvfb did not start; see xvfb.log")
                    time.sleep(0.05)
                with (artifacts / "gui-smoke.log").open("w") as log:
                    app = subprocess.Popen([str(binary), str(repo)], env=env, stdout=log, stderr=log)
                    try:
                        time.sleep(8)
                        if app.poll() is not None:
                            raise RuntimeError(f"GUI exited with code {app.returncode}; see gui-smoke.log")
                        if image_preview or line_selection:
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
t.XTestFakeMotionEvent(d, -1, 80, 485, 0)
t.XTestFakeButtonEvent(d, 1, 1, 0)
t.XTestFakeButtonEvent(d, 1, 0, 20)
x.XFlush(d)
x.XCloseDisplay(d)
'''
                            subprocess.run([sys.executable, "-c", click], env=env, check=True)
                            time.sleep(2)
                        if line_selection:
                            select = click.split("t.XTestFakeMotionEvent(d, -1, 80, 485, 0)")[0] + '''import time
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
press(600, 544)
control = x.XKeysymToKeycode(d, 0xffe3)
t.XTestFakeKeyEvent(d, control, 1, 0)
press(600, 582)
t.XTestFakeKeyEvent(d, control, 0, 0)
press(300, 482)
x.XCloseDisplay(d)'''
                            if split_selection:
                                select = select.replace("press(600, 544)", "press(1020, 451)\npress(1050, 544)").replace("press(600, 582)", "press(1050, 582)")
                            subprocess.run([sys.executable, "-c", select], env=env, check=True)
                            time.sleep(2)
                            png_from_xwd(root / "Xvfb_screen0", artifacts / "gui-smoke.png")
                            indexed = subprocess.check_output(["git", "-C", str(repo), "show", ":file.txt"], env=env)
                            assert indexed == b"base\nfirst\nlast\n", f"Incorrect selected index content: {indexed!r}"
                            assert (repo / "file.txt").read_bytes() == b"base\nfirst\nmiddle\nlast\n"
                        if image_preview:
                            png_from_xwd(root / "Xvfb_screen0", artifacts / "gui-smoke.png")
                            raw = (root / "Xvfb_screen0").read_bytes()
                            header = struct.unpack(">25I", raw[:100])
                            pixels = raw[header[0] + header[19] * 12:]
                            # Red before / blue after must both reach the rendered diff area.
                            bottom = pixels[470 * header[12]:730 * header[12]]
                            colors = [bottom[i:i+3] for i in range(0, len(bottom), 4)]
                            assert colors.count(bytes((0, 0, 255))) > 100, "Before image was not rendered"
                            assert colors.count(bytes((255, 0, 0))) > 100, "After image was not rendered"
                        png_from_xwd(root / "Xvfb_screen0", artifacts / "gui-smoke.png")
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
                            else "GUI remained running for 8 seconds; framebuffer captured")
                    finally:
                        if app.poll() is None:
                            app.terminate()
                            app.wait(timeout=5)
            finally:
                server.terminate()
                server.wait(timeout=5)


if __name__ == "__main__":
    main()
