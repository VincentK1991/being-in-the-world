#!/usr/bin/env python3
"""Check that the documentation points at things that exist in this repository.

  python3 scripts/docs/check_paths.py

Checks every page under apps/studio/src/content/docs:
  - <Src file="..." symbol="..."/>: the file exists and contains the symbol; <Src dir="..."/>: the directory exists;
  - <Adr n=".." slug=".."/>: docs/adr/NNNN-slug.md exists;
  - inline code that looks like a repository path (`crates/...`, `plugins/...`, `scripts/...`, ...):
    the path exists (globs and placeholders are skipped);
  - every diagram name used by <Diagram name=".."/> exists as src/assets/diagrams/<name>.svg.

This runs on the host and in CI, not inside the UI image build, which only contains apps/studio.
Exit status 1 when anything is wrong.
"""
import pathlib, re, sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
PAGES = ROOT / 'apps/studio/src/content/docs'
DIAGRAMS = ROOT / 'apps/studio/src/assets/diagrams'
TOP = ('crates', 'plugins', 'scripts', 'deploy', 'docs', 'sidecars', 'apps', 'tests')
PATH_LIKE = re.compile(r'^(?:' + '|'.join(TOP) + r')/[\w.\-/]+$')
FENCE = re.compile(r'^(```|~~~)')
ATTR = r'(\w+)=(?:"([^"]*)"|\{([^}]*)\})'


def attributes(tag):
    return {name: (quoted if quoted else braced) for name, quoted, braced in re.findall(ATTR, tag)}


def main():
    problems, checked = [], 0
    for page in sorted(PAGES.rglob('*.md*')):
        rel = page.relative_to(ROOT)
        text = page.read_text()
        in_fence = False
        for number, line in enumerate(text.splitlines(), 1):
            if FENCE.match(line.strip()):
                in_fence = not in_fence
            where = f'{rel}:{number}'
            for tag in re.findall(r'<Src\b[^>]*/?>', line):
                a = attributes(tag); checked += 1
                if a.get('dir'):
                    if not (ROOT / a['dir']).is_dir():
                        problems.append(f'{where}: <Src> directory does not exist: {a["dir"]}')
                    continue
                target = ROOT / a.get('file', '')
                if not a.get('file') or not target.is_file():
                    problems.append(f'{where}: <Src> file does not exist: {a.get("file")}')
                elif a.get('symbol') and a['symbol'] not in target.read_text(errors='replace'):
                    problems.append(f'{where}: symbol "{a["symbol"]}" not found in {a["file"]}')
            for tag in re.findall(r'<Adr\b[^>]*/?>', line):
                a = attributes(tag); checked += 1
                path = ROOT / 'docs/adr' / f'{int(a.get("n", "0")):04d}-{a.get("slug", "")}.md'
                if not path.is_file():
                    problems.append(f'{where}: ADR does not exist: {path.relative_to(ROOT)}')
            for tag in re.findall(r'<Diagram\b[^>]*/?>', line):
                a = attributes(tag); checked += 1
                if not (DIAGRAMS / f'{a.get("name", "")}.svg').is_file():
                    problems.append(f'{where}: diagram "{a.get("name")}" is not in src/assets/diagrams/')
            if in_fence:
                continue
            for code in re.findall(r'`([^`\n]+)`', line):
                candidate = code.strip().rstrip('/')
                if PATH_LIKE.match(candidate) and '*' not in candidate and '<' not in candidate:
                    checked += 1
                    if not (ROOT / candidate).exists():
                        problems.append(f'{where}: path does not exist: {code}')
    for problem in problems:
        print('FAIL', problem)
    print(f'docs paths: {checked} references checked, {len(problems)} problem(s)')
    sys.exit(1 if problems else 0)


main()
