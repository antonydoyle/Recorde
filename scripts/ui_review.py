"""Capture themes and scales using the isolated, supervised GUI smoke fixture.

Run via linux_task.py. Artifacts and fixture repositories stay outside the source tree.
Default fixtures retain dark/light, compact and welcome captures for Classic.
--siege uses Siege21 for those same fixtures. --theme-matrix captures all four
palette/mode combinations at native scale. --dpi-matrix captures all four at
100%, 125%, 150% and 200%. --review adds native inspector feature acceptance
(requires RECORDE_UI_INSPECTOR); the welcome screen keeps startup-only inspection.
"""
import sys
from pathlib import Path
from gui_smoke import main


def variants(options):
    if "--dpi-matrix" in options:
        scales = (1.0, 1.25, 1.5, 2.0)
    else:
        scales = (1.0,)
    if "--dpi-matrix" in options or "--theme-matrix" in options:
        return [([f"--dpi={scale}", *(["--siege"] if siege else []), *(["--light"] if light else [])],
                 f"{'siege' if siege else 'classic'}-{'light' if light else 'dark'}-{int(scale * 100)}")
                for scale in scales for siege in (False, True) for light in (False, True)]
    palette_args = ["--siege"] if "--siege" in options else []
    prefix = "siege-" if palette_args else ""
    return [(palette_args + extra, prefix + label) for extra, label in [
        ([], "dark"), (["--light"], "light"), (["--compact"], "compact"), (["--light", "--welcome"], "welcome")]]


if __name__ == "__main__":
    binary, artifacts = sys.argv[1:3]
    options = set(sys.argv[3:])
    unknown = options - {"--siege", "--theme-matrix", "--dpi-matrix", "--review"}
    if unknown:
        raise SystemExit(f"Unknown review options: {', '.join(sorted(unknown))}")
    for extra, label in variants(options):
        review_args = ["--review"] if "--review" in options and "--welcome" not in extra else []
        sys.argv = [sys.argv[0], binary, str(Path(artifacts) / label),
                    "--design-review", *extra, *review_args]
        main()
