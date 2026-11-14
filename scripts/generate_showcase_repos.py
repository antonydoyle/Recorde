#!/usr/bin/env python3
"""Generate busy, realistic-looking fake repositories for screenshots.

The repositories model a small product team using a Git Flow style process:
many concurrent feature branches, pull-request merges, release and hotfix
branches, tags, remote-tracking branches that are ahead or behind, and a
dirty working tree. Every name, person and address is fictional. Output is
deterministic for a given seed, so marketing captures can be reproduced.

    python scripts/generate_showcase_repos.py <output-directory>

The output directory must not exist (or be empty). Write it outside the
source tree; generated repositories never belong in version control.
"""
from __future__ import annotations

import argparse
import os
import random
import subprocess
import sys
from datetime import datetime, timedelta, timezone
from pathlib import Path

PEOPLE = [
    ("Maya Chen", "maya.chen"),
    ("Tomás Herrera", "tomas.herrera"),
    ("Priya Natarajan", "priya.n"),
    ("Jonas Weber", "jonas.weber"),
    ("Amara Okafor", "amara.okafor"),
    ("Liam Byrne", "liam.byrne"),
    ("Sofia Rossi", "sofia.rossi"),
    ("Kenji Watanabe", "kenji.w"),
    ("Hannah Lindqvist", "hannah.l"),
    ("Daniel Mensah", "daniel.mensah"),
]
MAINTAINERS = [0, 3, 6]

AREAS = [
    "billing", "search", "dashboard", "settings", "editor", "uploads",
    "notifications", "onboarding", "auth", "reports", "sharing", "comments",
    "projects", "members", "integrations", "exports", "calendar", "timeline",
]
KINDS = [
    "filters", "bulk-actions", "empty-states", "permissions", "pagination",
    "keyboard-nav", "caching", "retry", "audit-trail", "drafts", "previews",
    "sorting", "offline-mode", "webhooks", "live-updates", "templates",
]
FIX_TOPICS = [
    "session-timeout", "safari-upload", "timezone-offsets", "flaky-e2e",
    "memory-leak", "double-submit", "focus-trap", "scroll-jank", "csv-quotes",
    "token-refresh", "rtl-layout", "slow-query",
]
CHORES = ["upgrade-react", "ci-cache", "lint-rules", "bump-deps", "storybook", "vite-config"]

FEATURE_MESSAGES = [
    "Add {kind} to {area}",
    "Wire {area} {kind} into the API client",
    "Handle empty {area} lists in {kind}",
    "Fix {kind} state not resetting after {area} reload",
    "Refactor {area} {kind} hooks",
    "Add tests for {area} {kind}",
    "Polish {area} {kind} spacing and focus states",
    "Address review feedback on {kind}",
    "Cache {area} responses for {kind}",
    "Show a loading skeleton while {area} {kind} resolve",
    "Rename {kind} props for clarity",
    "Guard {area} {kind} behind a feature flag",
    "Improve {kind} error messages",
    "Make {area} {kind} keyboard accessible",
    "Split {area} {kind} into smaller components",
    "Debounce {area} {kind} requests",
]
DEVELOP_MESSAGES = [
    "Update dependencies",
    "Tidy up shared button styles",
    "Fix typo in onboarding copy",
    "Bump TypeScript to 5.9",
    "Speed up CI by caching node_modules",
    "Remove unused analytics events",
    "Document the release checklist",
    "Fix flaky date picker test",
    "Increase API client timeout",
    "Extract shared table component",
    "Add Storybook stories for form fields",
    "Lower bundle size by lazy loading charts",
]
FIX_MESSAGES = [
    "Reproduce {topic} in a failing test",
    "Fix {topic}",
    "Add regression coverage for {topic}",
    "Clean up after {topic} fix",
]
WORDS = {
    "verb": ["load", "resolve", "build", "format", "merge", "parse", "select", "group", "sync", "validate"],
    "noun": ["Filter", "Entry", "Range", "Member", "Invoice", "Draft", "Preview", "Rule", "Event", "Report"],
    "field": ["name", "range", "owner", "status", "cursor", "locale", "limit", "scope"],
}


def slug_words(slug: str) -> str:
    return slug.replace("-", " ")


class Repo:
    def __init__(self, name: str, seed: int, target_commits: int, active: int, start: datetime, end: datetime):
        self.name = name
        self.rng = random.Random(seed)
        self.target = target_commits
        self.active_target = active
        self.time = start
        self.end = end
        self.out: list[bytes] = []
        self.mark = 0
        self.rev = 0
        self.commits = 0
        self.branches: dict[str, dict] = {}
        self.kept: dict[str, int] = {}  # local refs that outlive a merge
        self.remote: dict[str, int] = {}
        self.tags: list[tuple[str, int, str]] = []
        self.pr = 40
        self.version = [1, 0, 0]
        self.used_features: set[str] = set()
        self.develop_since_release = 0

    # ----- content -----
    def snippet(self) -> str:
        r = self.rng
        verb, noun, field = r.choice(WORDS["verb"]), r.choice(WORDS["noun"]), r.choice(WORDS["field"])
        name = verb + noun
        return (
            f"export function {name}(input: {noun}Input): {noun}Result {{\n"
            f"  const {field} = normalise(input.{field});\n"
            f"  if (!{field}) {{\n"
            f'    return {{ ok: false, reason: "missing-{field}" }};\n'
            f"  }}\n"
            f"  return {{ ok: true, value: {field} }};\n"
            f"}}\n"
        )

    def file(self, header: str, blocks: int = 2) -> tuple:
        self.rev += 1
        return (self.rev, (header,) + tuple(self.snippet() for _ in range(blocks)))

    def edit(self, value: tuple | None, header: str) -> tuple:
        self.rev += 1
        if value is None:
            return (self.rev, (header, self.snippet()))
        blocks = list(value[1])
        if len(blocks) > 7:
            del blocks[1]
        if len(blocks) > 1 and self.rng.random() < 0.35:
            blocks[self.rng.randrange(1, len(blocks))] = self.snippet()
        else:
            blocks.append(self.snippet())
        return (self.rev, tuple(blocks))

    def text(self, header: str, body: str) -> tuple:
        self.rev += 1
        return (self.rev, (header, body))

    # ----- time -----
    def working_times(self, count: int) -> list[int]:
        """Evenly spread timestamps in working hours, mostly on weekdays."""
        start, span = self.time.timestamp(), (self.end - self.time).total_seconds()
        times = []
        while len(times) < count:
            t = datetime.fromtimestamp(start + self.rng.random() * span, timezone.utc)
            if 8 <= t.hour < 19 and (t.weekday() < 5 or self.rng.random() < 0.05):
                times.append(int(t.timestamp()))
        times.sort()
        return times

    def tick(self) -> int:
        if not hasattr(self, "times"):
            # A small surplus keeps the final commits close to the end date.
            self.times = self.working_times(self.target + 4)
            self.last = 0
        when = self.times[self.commits] if self.commits < len(self.times) else self.last + 60
        self.last = max(when, self.last + 20)
        return self.last

    # ----- fast-import -----
    def emit(self, ref: str, person: int, message: str, parent: dict | None, tree: dict,
             parents: list[int]) -> int:
        when = self.tick()
        self.mark += 1
        self.commits += 1
        name, user = PEOPLE[person]
        ident = f"{name} <{user}@example.com> {when} +0000".encode()
        msg = message.encode()
        lines = [f"commit {ref}\nmark :{self.mark}\n".encode(),
                 b"author " + ident + b"\n", b"committer " + ident + b"\n",
                 f"data {len(msg)}\n".encode(), msg, b"\n"]
        if parents:
            lines.append(f"from :{parents[0]}\n".encode())
            for extra in parents[1:]:
                lines.append(f"merge :{extra}\n".encode())
        base = parent or {}
        for path, value in sorted(tree.items()):
            if base.get(path) is value:
                continue
            data = "\n".join(value[1]).encode()
            lines.append(f"M 100644 inline {path}\ndata {len(data)}\n".encode())
            lines.append(data + b"\n")
        for path in sorted(set(base) - set(tree)):
            lines.append(f"D {path}\n".encode())
        lines.append(b"\n")
        self.out.extend(lines)
        return self.mark

    def commit(self, branch: str, person: int, message: str, changes: dict) -> None:
        b = self.branches[branch]
        tree = dict(b["tree"])
        tree.update(changes)
        b["tip"] = self.emit(f"refs/heads/{branch}", person, message, b["tree"], tree, [b["tip"]])
        b["tree"] = tree
        b["count"] += 1

    def merge(self, target: str, source: str, person: int, message: str) -> None:
        t, s = self.branches[target], self.branches[source]
        tree = dict(t["tree"])
        for path, value in s["tree"].items():
            if path not in tree or value[0] > tree[path][0]:
                tree[path] = value
        t["tip"] = self.emit(f"refs/heads/{target}", person, message, t["tree"], tree, [t["tip"], s["tip"]])
        t["tree"] = tree

    def branch(self, name: str, base: str, kind: str, owner: int, **extra) -> None:
        b = self.branches[base]
        self.branches[name] = {"tip": b["tip"], "tree": b["tree"], "kind": kind, "owner": owner,
                               "count": 0, "files": [], **extra}

    # ----- simulation -----
    def seed_tree(self) -> None:
        base = {
            "README.md": self.text("# " + self.name, "A fictional product used for Recorde screenshots.\n"),
            "package.json": self.text("{", f'  "name": "{self.name}",\n  "version": "1.0.0"\n}}'),
            "CHANGELOG.md": self.text("# Changelog", "## 1.0.0\n- First release\n"),
            "tsconfig.json": self.text("{", '  "compilerOptions": { "strict": true }\n}'),
            "src/app/routes.ts": self.file("// Application routes"),
            "src/app/main.tsx": self.file("// Entry point"),
            "src/lib/api.ts": self.file("// API client"),
            "src/lib/format.ts": self.file("// Formatting helpers"),
            "src/lib/flags.ts": self.file("// Feature flags", 1),
            "src/ui/Button.tsx": self.file("// Button"),
            "src/ui/Table.tsx": self.file("// Table"),
            "src/ui/Dialog.tsx": self.file("// Dialog"),
        }
        self.branches["main"] = {"tip": 0, "tree": {}, "kind": "main", "owner": 0, "count": 0, "files": []}
        self.mark += 1
        self.commits += 1
        when = self.tick()
        name, user = PEOPLE[0]
        ident = f"{name} <{user}@example.com> {when} +0000".encode()
        msg = b"Initial commit"
        self.out.append(f"commit refs/heads/main\nmark :{self.mark}\n".encode())
        self.out += [b"author " + ident + b"\n", b"committer " + ident + b"\n",
                     f"data {len(msg)}\n".encode(), msg, b"\n"]
        for path, value in sorted(base.items()):
            data = "\n".join(value[1]).encode()
            self.out += [f"M 100644 inline {path}\ndata {len(data)}\n".encode(), data, b"\n"]
        self.out.append(b"\n")
        self.branches["main"].update(tip=self.mark, tree=base)
        for message in ["Set up linting and formatting", "Add CI workflow", "Add base UI components"]:
            self.commit("main", 0, message, {"src/ui/Button.tsx": self.edit(base["src/ui/Button.tsx"], "// Button")})
        self.branch("develop", "main", "develop", 0)
        self.tags.append(("v1.0.0", self.branches["main"]["tip"], "First public release"))

    def new_feature(self) -> str | None:
        r = self.rng
        roll = r.random()
        for _ in range(40):
            if roll < 0.72:
                area, kind = r.choice(AREAS), r.choice(KINDS)
                name, title = f"feature/{area}-{kind}", (area, slug_words(kind))
            elif roll < 0.9:
                topic = r.choice(FIX_TOPICS)
                name, title = f"fix/{topic}", (None, slug_words(topic))
            else:
                chore = r.choice(CHORES)
                name, title = f"chore/{chore}", (None, slug_words(chore))
            if name not in self.used_features and name not in self.branches:
                break
        else:
            return None
        self.used_features.add(name)
        owner = r.randrange(len(PEOPLE))
        length = r.randint(3, 14) if name.startswith("feature/") else r.randint(1, 4)
        self.branch(name, "develop", "feature", owner, title=title, length=length)
        return name

    def feature_commit(self, name: str) -> None:
        b = self.branches[name]
        area, words = b["title"]
        slug = name.split("/", 1)[1]
        folder = f"src/features/{slug}"
        if name.startswith("feature/"):
            message = self.rng.choice(FEATURE_MESSAGES).format(area=area, kind=words)
            if b["count"] == 0:
                message = f"Add {words} to {area}"
        elif name.startswith("fix/"):
            message = FIX_MESSAGES[min(b["count"], len(FIX_MESSAGES) - 1)].format(topic=words)
        else:
            message = self.rng.choice(["Update", "Configure", "Adjust"]) + f" {words}"
        candidates = [f"{folder}/index.ts", f"{folder}/{slug}.tsx", f"{folder}/{slug}.test.ts", f"{folder}/api.ts"]
        if not name.startswith("feature/"):
            candidates = ["src/lib/api.ts", "src/lib/format.ts", "src/ui/Table.tsx", "package.json"]
        changes = {}
        for path in self.rng.sample(candidates, self.rng.randint(1, min(3, len(candidates)))):
            if path.endswith(".json"):
                continue
            changes[path] = self.edit(b["tree"].get(path), f"// {path}")
        if not changes:
            changes[candidates[0]] = self.edit(b["tree"].get(candidates[0]), f"// {candidates[0]}")
        person = b["owner"] if self.rng.random() < 0.85 else self.rng.randrange(len(PEOPLE))
        self.commit(name, person, message, changes)

    def finish_feature(self, name: str) -> None:
        reviewer = self.rng.choice(MAINTAINERS)
        self.pr += self.rng.randint(1, 4)
        if self.rng.random() < 0.8:
            message = f"Merge pull request #{self.pr} from atlas/{name}\n\n{self.branches[name]['title'][1].capitalize()}"
        else:
            message = f"Merge branch '{name}' into develop"
        self.merge("develop", name, reviewer, message)
        self.develop_since_release += 1
        b = self.branches.pop(name)
        if self.rng.random() < 0.18:
            self.kept[name] = b["tip"]
        if self.rng.random() < 0.3:
            self.remote[name] = b["tip"]

    def start_release(self) -> None:
        self.version = [self.version[0], self.version[1] + 1, 0]
        if self.version[1] >= 8:
            self.version = [self.version[0] + 1, 0, 0]
        v = ".".join(map(str, self.version))
        name = f"release/{v}"
        self.branch(name, "develop", "release", self.rng.choice(MAINTAINERS), version=v, length=self.rng.randint(2, 5))
        b = self.branches[name]
        b["tree"] = dict(b["tree"])
        self.commit(name, b["owner"], f"Bump version to {v}",
                    {"package.json": self.text("{", f'  "name": "{self.name}",\n  "version": "{v}"\n}}')})
        self.develop_since_release = 0

    def release_commit(self, name: str) -> None:
        b = self.branches[name]
        v = b["version"]
        if b["count"] == 1:
            notes = b["tree"]["CHANGELOG.md"]
            self.commit(name, b["owner"], f"Update changelog for {v}",
                        {"CHANGELOG.md": self.text("# Changelog", f"## {v}\n- Improvements and fixes\n\n" + notes[1][1])})
        else:
            area = self.rng.choice(AREAS)
            path = "src/lib/format.ts" if self.rng.random() < 0.5 else "src/ui/Dialog.tsx"
            self.commit(name, self.rng.randrange(len(PEOPLE)), f"Fix {area} regression found in {v} QA",
                        {path: self.edit(b["tree"].get(path), f"// {path}")})

    def finish_release(self, name: str) -> None:
        b = self.branches[name]
        v = b["version"]
        kind = b["kind"]
        self.merge("main", name, b["owner"], f"Merge branch '{name}'")
        self.tags.append((f"v{v}", self.branches["main"]["tip"], f"Version {v}"))
        self.merge("develop", name, b["owner"], f"Merge branch '{name}' into develop")
        self.branches.pop(name)
        if kind == "release":
            self.kept[name] = b["tip"]
            self.remote[name] = b["tip"]

    def start_hotfix(self) -> None:
        major, minor, patch = self.version
        self.version = [major, minor, patch + 1]
        v = ".".join(map(str, self.version))
        name = f"hotfix/{v}"
        topic = self.rng.choice(FIX_TOPICS)
        self.branch(name, "main", "hotfix", self.rng.choice(MAINTAINERS), version=v, length=self.rng.randint(1, 2), topic=topic)
        self.commit(name, self.branches[name]["owner"], f"Fix {slug_words(topic)} in production",
                    {"src/lib/api.ts": self.edit(self.branches[name]["tree"].get("src/lib/api.ts"), "// API client")})

    def develop_commit(self) -> None:
        path = self.rng.choice(["src/app/routes.ts", "src/lib/api.ts", "src/ui/Button.tsx", "src/ui/Table.tsx", "src/lib/flags.ts"])
        b = self.branches["develop"]
        message = self.rng.choice([m for m in DEVELOP_MESSAGES if m != getattr(self, "last_develop", None)])
        self.last_develop = message
        self.commit("develop", self.rng.randrange(len(PEOPLE)), message,
                    {path: self.edit(b["tree"].get(path), f"// {path}")})
        self.develop_since_release += 1

    def simulate(self, finale: bool) -> None:
        r = self.rng
        self.seed_tree()
        reserve = 6 + (len(FINALE) if finale else 0)
        while self.commits < self.target - reserve:
            features = [n for n, b in self.branches.items() if b["kind"] == "feature"]
            special = [n for n, b in self.branches.items() if b["kind"] in ("release", "hotfix")]
            late = self.commits > self.target * 0.92
            wanted = self.active_target + (4 if late else 0)
            if len(features) < wanted and r.random() < 0.35:
                created = self.new_feature()
                if created:
                    self.feature_commit(created)
                continue
            for name in special:
                b = self.branches[name]
                if b["count"] >= b["length"]:
                    self.finish_release(name)
                    break
            else:
                roll = r.random()
                if special and roll < 0.25:
                    name = r.choice(special)
                    if name.startswith("release/"):
                        self.release_commit(name)
                    else:
                        self.commit(name, self.branches[name]["owner"], "Add regression test for " + slug_words(self.branches[name]["topic"]),
                                    {"src/lib/api.ts": self.edit(self.branches[name]["tree"].get("src/lib/api.ts"), "// API client")})
                elif not special and self.develop_since_release > 24 and roll < 0.3 and not late:
                    self.start_release()
                elif not special and roll < 0.012 and not late:
                    self.start_hotfix()
                elif features and roll < 0.75:
                    ready = [n for n in features if self.branches[n]["count"] >= self.branches[n]["length"]]
                    if ready and r.random() < (0.3 if late else 0.55):
                        self.finish_feature(r.choice(ready))
                    else:
                        # Recent branches get most of the work.
                        name = features[min(int(r.expovariate(0.35)), len(features) - 1) * -1 - 1]
                        if r.random() < 0.04 and self.branches[name]["count"] > 2:
                            self.merge(name, "develop", self.branches[name]["owner"], f"Merge branch 'develop' into {name}")
                        else:
                            self.feature_commit(name)
                else:
                    self.develop_commit()
        for name in [n for n, b in self.branches.items() if b["kind"] in ("release", "hotfix")]:
            self.finish_release(name)
        if finale:
            self.finale()

    def finale(self) -> None:
        """Script the last fortnight so the top of the graph shows how history
        branches and joins: branches spurring off develop, pull requests merging
        back, develop merged into a feature, and a branch started after a merge."""
        maintainer = MAINTAINERS[1]
        for step in FINALE:
            kind, name = step[0], step[1]
            if kind == "branch":
                self.branch(name, "develop", "feature", step[2], title=("", ""), length=99)
            elif kind == "commit":
                slug = name.split("/", 1)[1]
                path = f"src/features/{slug}/{slug}.tsx"
                b = self.branches[name]
                self.commit(name, b["owner"], step[2], {path: self.edit(b["tree"].get(path), f"// {path}")})
            elif kind == "develop":
                self.commit("develop", maintainer, name,
                            {"package.json": self.text("{", f'  "name": "{self.name}",\n  "version": "next"\n}}')})
            elif kind == "sync":
                self.merge(name, "develop", self.branches[name]["owner"], f"Merge branch 'develop' into {name}")
            elif kind == "pr":
                self.pr += 3
                self.merge("develop", name, maintainer, f"Merge pull request #{self.pr} from atlas/{name}\n\n{step[2]}")
                self.branches.pop(name)

    # ----- output -----
    def write(self, directory: Path, checkout: str) -> dict:
        directory.mkdir(parents=True)
        env = dict(os.environ, GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull)

        def git(*args: str, data: bytes | None = None) -> str:
            result = subprocess.run(["git", "-C", str(directory), *args], input=data, env=env,
                                    capture_output=True)
            if result.returncode:
                raise SystemExit(f"git {args[0]} failed: {result.stderr.decode(errors='replace')[-2000:]}")
            return result.stdout.decode()

        git("init", "-q", "-b", checkout)
        git("config", "core.autocrlf", "false")
        git("config", "user.name", PEOPLE[1][0])
        git("config", "user.email", f"{PEOPLE[1][1]}@example.com")
        stream = list(self.out)
        for name, mark in self.kept.items():
            stream.append(f"reset refs/heads/{name}\nfrom :{mark}\n\n".encode())
        for name, b in self.branches.items():
            stream.append(f"reset refs/remotes/origin/{name}\nfrom :{b['tip']}\n\n".encode())
        for name, mark in self.remote.items():
            stream.append(f"reset refs/remotes/origin/{name}\nfrom :{mark}\n\n".encode())
        for tag, mark, message in self.tags:
            msg = message.encode()
            ident = f"{PEOPLE[0][0]} <{PEOPLE[0][1]}@example.com> {int(self.end.timestamp())} +0000".encode()
            stream += [f"tag {tag}\nfrom :{mark}\n".encode(), b"tagger " + ident + b"\n",
                       f"data {len(msg)}\n".encode(), msg, b"\n"]
        git("fast-import", "--quiet", data=b"".join(stream))
        # fast-import keeps every branch it committed to; merged branches that
        # a team would have deleted are removed here.
        wanted = {f"refs/heads/{name}" for name in list(self.branches) + list(self.kept)}
        stale = [ref for ref in git("for-each-ref", "--format=%(refname)", "refs/heads").split() if ref not in wanted]
        git("update-ref", "--stdin", data="".join(f"delete {ref}\n" for ref in stale).encode())
        git("config", "remote.origin.url", f"https://git.example.com/atlas/{self.name}.git")
        git("config", "remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*")
        for name in list(self.branches) + list(self.kept):
            git("config", f"branch.{name}.remote", "origin")
            git("config", f"branch.{name}.merge", f"refs/heads/{name}")
        git("checkout", "-q", "-f", checkout)
        return {"commits": self.commits, "local": len(self.branches) + len(self.kept),
                "remote": len(self.branches) + len(self.remote), "tags": len(self.tags)}


# The scripted end of atlas-web's history, oldest first. Owners index PEOPLE.
FINALE = [
    ("branch", "fix/upload-progress", 7),
    ("commit", "fix/upload-progress", "Fix upload progress stalling at 99%"),
    ("branch", "feature/dark-mode-charts", 6),
    ("commit", "feature/dark-mode-charts", "Theme chart axes and gridlines"),
    ("branch", "feature/command-palette", 0),
    ("commit", "feature/command-palette", "Add command palette shell"),
    ("commit", "feature/dark-mode-charts", "Use theme tokens for chart series"),
    ("develop", "Update dependencies"),
    ("commit", "feature/command-palette", "List recent projects in the command palette"),
    ("commit", "fix/upload-progress", "Add retry for interrupted uploads"),
    ("commit", "feature/dark-mode-charts", "Fix tooltip contrast in dark mode"),
    ("pr", "feature/dark-mode-charts", "Dark mode for charts"),
    ("branch", "feature/billing-annual-plans", 4),
    ("commit", "feature/billing-annual-plans", "Add annual plan toggle to pricing"),
    ("commit", "feature/command-palette", "Add keyboard navigation to command results"),
    ("sync", "feature/command-palette"),
    ("commit", "feature/billing-annual-plans", "Prorate upgrades to annual plans"),
    ("pr", "fix/upload-progress", "Fix stalled uploads"),
    ("commit", "feature/command-palette", "Add fuzzy matching to the command palette"),
    ("branch", "feature/onboarding-tour", 9),
    ("commit", "feature/onboarding-tour", "Add onboarding tour scaffold"),
    ("commit", "feature/billing-annual-plans", "Show savings badge on annual plans"),
]


def dirty_tree(directory: Path) -> None:
    palette = directory / "src/features/command-palette/command-palette.tsx"
    palette.write_text(palette.read_text(encoding="utf-8") + (
        "\nexport function scoreMatch(query: string, label: string): number {\n"
        "  const q = query.toLowerCase();\n"
        "  let score = 0;\n"
        "  for (const char of label.toLowerCase()) {\n"
        "    if (q.includes(char)) score += 1;\n"
        "  }\n"
        "  return score / Math.max(label.length, 1);\n"
        "}\n"), encoding="utf-8", newline="\n")
    (directory / "src/features/command-palette/fuzzy.test.ts").write_text(
        'import { scoreMatch } from "./command-palette";\n\n'
        'test("ranks exact prefixes first", () => {\n'
        '  expect(scoreMatch("set", "Settings")).toBeGreaterThan(scoreMatch("set", "Reset"));\n'
        "});\n", encoding="utf-8", newline="\n")
    changelog = directory / "CHANGELOG.md"
    changelog.write_text("# Changelog\n## Unreleased\n- Command palette with fuzzy matching\n\n"
                         + changelog.read_text(encoding="utf-8").split("\n", 1)[1], encoding="utf-8", newline="\n")


TOKENS = """// Colour tokens shared by every product surface.
export const color = {
  primary: "#2563eb",
  primaryHover: "#1d4ed8",
  surface: "#ffffff",
  surfaceRaised: "#f8fafc",
  border: "#e2e8f0",
  text: "#0f172a",
  textMuted: "#64748b",
  success: "#16a34a",
  danger: "#dc2626",
};
"""


def merge_conflict(directory: Path, when: datetime) -> None:
    """Leave a merge in progress with a conflicted file, for the conflict editor."""
    env = dict(os.environ, GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull)
    tokens = directory / "src/tokens/color.ts"

    def git(*args: str, person: int = 0, offset: int = 0, check: bool = True) -> None:
        name, user = PEOPLE[person]
        stamp = (when + timedelta(minutes=offset)).isoformat()
        run_env = dict(env, GIT_AUTHOR_NAME=name, GIT_AUTHOR_EMAIL=f"{user}@example.com",
                       GIT_COMMITTER_NAME=name, GIT_COMMITTER_EMAIL=f"{user}@example.com",
                       GIT_AUTHOR_DATE=stamp, GIT_COMMITTER_DATE=stamp)
        result = subprocess.run(["git", "-C", str(directory), *args], env=run_env, capture_output=True)
        if check and result.returncode:
            raise SystemExit(f"git {args[0]} failed: {result.stderr.decode(errors='replace')}")

    def write(text: str) -> None:
        tokens.parent.mkdir(parents=True, exist_ok=True)
        tokens.write_text(text, encoding="utf-8", newline="\n")

    write(TOKENS)
    git("add", "src/tokens/color.ts")
    git("commit", "-q", "-m", "Add colour tokens", person=3)
    git("checkout", "-q", "-b", "feature/brand-refresh")
    write(TOKENS.replace('"#2563eb"', '"#6366f1"').replace('"#1d4ed8"', '"#4f46e5"')
          .replace('"#f8fafc"', '"#f5f7ff"').replace('"#e2e8f0"', '"#e0e7ff"'))
    git("commit", "-q", "-am", "Refresh brand colours for the 2027 launch", person=6, offset=40)
    git("checkout", "-q", "main")
    write(TOKENS.replace('"#2563eb"', '"#1e40af"').replace('"#1d4ed8"', '"#1e3a8a"')
          .replace('"#64748b"', '"#475569"'))
    git("commit", "-q", "-am", "Darken primary and muted text for contrast", person=3, offset=75)
    git("merge", "--no-ff", "feature/brand-refresh", person=3, offset=90, check=False)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("output", type=Path)
    parser.add_argument("--seed", type=int, default=21)
    parser.add_argument("--commits", type=int, default=2600, help="approximate commits in the main repository")
    args = parser.parse_args()
    output = args.output.resolve()
    if output.exists() and any(output.iterdir()):
        parser.error(f"{output} is not empty")
    source = Path(__file__).resolve().parents[1]
    if output == source or source in output.parents:
        parser.error("write generated repositories outside the source tree")
    end = datetime(2026, 11, 13, 16, 40, tzinfo=timezone.utc)
    plans = [
        ("atlas-web", args.commits, 7, datetime(2025, 6, 2, 9, tzinfo=timezone.utc),
         True, "feature/command-palette"),
        ("atlas-api", max(args.commits // 4, 120), 4, datetime(2025, 9, 1, 9, tzinfo=timezone.utc), False, "develop"),
        ("design-system", max(args.commits // 8, 80), 3, datetime(2026, 1, 12, 9, tzinfo=timezone.utc), False, "main"),
    ]
    for index, (name, commits, active, start, finale, checkout) in enumerate(plans):
        repo = Repo(name, args.seed + index, commits, active, start, end)
        repo.simulate(finale)
        summary = repo.write(output / name, checkout)
        if name == "atlas-web":
            # The checked-out branch has a commit that is not pushed yet,
            # and the working tree has changes in progress.
            env = dict(os.environ, GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull)
            subprocess.run(["git", "-C", str(output / name), "update-ref", f"refs/remotes/origin/{checkout}",
                            f"{checkout}~1"], env=env, check=True, capture_output=True)
            dirty_tree(output / name)
        if name == "design-system":
            merge_conflict(output / name, end - timedelta(hours=3))
        print(f"{name}: {summary['commits']} commits, {summary['local']} local branches, "
              f"{summary['remote']} remote branches, {summary['tags']} tags")
    return 0


if __name__ == "__main__":
    sys.exit(main())
