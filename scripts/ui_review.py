"""Capture both themes using the isolated, supervised GUI smoke fixture.

Run via linux_task.py. Artifacts and fixture repositories stay outside the source tree.
"""
import sys
from pathlib import Path
from gui_smoke import main

if __name__ == "__main__":
    binary, artifacts = sys.argv[1:3]
    for extra, label in [( [], "dark"), (["--light"], "light"), (["--compact"], "compact"), (["--light", "--welcome"], "welcome")]:
        sys.argv = [sys.argv[0], binary, str(Path(artifacts) / label),
                    "--design-review", *extra]
        main()
