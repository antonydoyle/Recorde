"""Reject accidental direct background Git/SSH launches before compiling or testing."""
from pathlib import Path
import re

root = Path(__file__).resolve().parents[1]
errors = []
direct = re.compile(r'Command\s*::\s*new\s*\(\s*"(?:git|ssh-add|ssh-keygen)(?:\.exe)?"', re.IGNORECASE)
for folder in (root / "src", root / "tests"):
    for path in folder.rglob("*.rs"):
        text = path.read_text(encoding="utf-8")
        for match in direct.finditer(text):
            line = text.count("\n", 0, match.start()) + 1
            errors.append(f"{path.relative_to(root)}:{line}: use the supervised background command helper")
main = (root / "src/main.rs").read_text(encoding="utf-8")
if re.search(r'cfg_attr\(\s*windows\s*,\s*windows_subsystem', main):
    errors.append("src/main.rs: GUI subsystem must exclude test executables")
if errors:
    raise SystemExit("\n".join(errors))
print("Process safety source check passed")
