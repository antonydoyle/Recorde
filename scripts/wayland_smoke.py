"""Native Wayland first-frame smoke against an existing compositor.

Run through linux_task.py. Uses an isolated config and repository, and stops only
its own app. The compositor is never started, changed or stopped by this script.
Usage: wayland_smoke.py RECORD_BINARY INSPECTOR_BINARY ARTIFACTS
"""
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time


def main():
    binary, inspector, artifacts = map(lambda arg: Path(arg).resolve(), sys.argv[1:4])
    artifacts.mkdir(parents=True, exist_ok=True)
    runtime = Path(os.environ.get("XDG_RUNTIME_DIR", "/run/user/1000"))
    display = os.environ.get("WAYLAND_DISPLAY", "wayland-0")
    if not (runtime / display).exists() and (Path("/mnt/wslg/runtime-dir") / display).exists():
        runtime = Path("/mnt/wslg/runtime-dir")
    assert (runtime / display).exists(), "An existing Wayland compositor is required"
    with tempfile.TemporaryDirectory(prefix="recorde-wayland-") as temporary:
        root = Path(temporary)
        repo = root / "fixture"
        repo.mkdir()
        env = os.environ.copy()
        env.pop("DISPLAY", None)  # A successful window cannot be an X11 fallback.
        env.pop("WAYLAND_SOCKET", None)
        env.update(XDG_RUNTIME_DIR=str(runtime), WAYLAND_DISPLAY=display,
                   XDG_CONFIG_HOME=str(root / "config"), XDG_DATA_HOME=str(root / "data"),
                   GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=str(root / "gitconfig"),
                   GIT_TERMINAL_PROMPT="0", RUST_LOG="warn")
        with socket.socket() as probe:
            probe.bind(("127.0.0.1", 0))
            address = f"127.0.0.1:{probe.getsockname()[1]}"
        env["EGUI_INSPECTION"] = address
        for args in [("init", "-b", "main"), ("config", "user.name", "Wayland Fixture"),
                     ("config", "user.email", "wayland@example.invalid")]:
            subprocess.run(["git", "-C", str(repo), *args], env=env, check=True, timeout=10, stdout=subprocess.DEVNULL)
        (repo / "file").write_text("first frame\n")
        subprocess.run(["git", "-C", str(repo), "add", "file"], env=env, check=True, timeout=10)
        subprocess.run(["git", "-C", str(repo), "-c", "commit.gpgsign=false", "commit", "-m", "Wayland first frame"], env=env, check=True, timeout=10, stdout=subprocess.DEVNULL)
        with (artifacts / "app.log").open("w") as log:
            app = subprocess.Popen([str(binary), str(repo)], env=env, stdout=log, stderr=log)
            try:
                (artifacts / "launch.json").write_text(json.dumps({"pid":app.pid, "backend":"Wayland", "socket":str(runtime / display), "inspection":address}, indent=2))
                deadline = time.monotonic() + 25
                while True:
                    assert app.poll() is None, "Wayland app exited before its first frame"
                    try:
                        with socket.create_connection(("127.0.0.1", int(address.rsplit(":", 1)[1])), timeout=0.2):
                            break
                    except OSError:
                        assert time.monotonic() < deadline, "Wayland app never reached creation callback"
                        time.sleep(0.1)
                time.sleep(2)
                subprocess.run([str(inspector), address, str(artifacts)], env=env, check=True, timeout=120)
                assert app.poll() is None, "Wayland app exited after rendering"
                print("Wayland native first frame, AccessKit tree and keyboard focus passed")
            finally:
                if app.poll() is None:
                    app.terminate()
                    try:
                        app.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        app.kill()
                        app.wait(timeout=5)


if __name__ == "__main__":
    main()
