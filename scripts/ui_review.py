"""Capture both themes using the isolated, supervised GUI smoke fixture.

Run via linux_task.py. Artifacts and fixture repositories stay outside the source tree.
"""
import sys
from pathlib import Path
from gui_smoke import main

if __name__ == "__main__":
    binary, artifacts = sys.argv[1:3]
    variants = [( [], "dark"), (["--light"], "light"), (["--compact"], "compact"), (["--light", "--welcome"], "welcome")]
    if "--dpi-matrix" in sys.argv[3:]:
        variants = [([f"--dpi={scale}", *(["--light"] if light else [])],
                     f"{'light' if light else 'dark'}-{int(scale * 100)}")
                    for scale in (1.0, 1.25, 1.5, 2.0) for light in (False, True)]
    for extra, label in variants:
        sys.argv = [sys.argv[0], binary, str(Path(artifacts) / label),
                    "--design-review", *extra]
        main()
