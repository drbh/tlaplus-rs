#!/usr/bin/env python3
"""Stage the playground dependency closure without development-only oracle crates.

The root workspace also contains adapters into an unpublished sibling checkout.
Copy only reachable crates, retaining their manifests and the root lockfile.
"""
import pathlib
import re
import shutil
import subprocess

root = pathlib.Path(__file__).resolve().parents[2]
stage = root / 'playground/.cache/browser-workspace'
stage.mkdir(parents=True, exist_ok=True)
pending = [root / 'tla-playground']
crates = set()
while pending:
    crate = pending.pop().resolve()
    if crate in crates:
        continue
    if crate.parent != root:
        raise RuntimeError(f'Playground dependency outside repository: {crate}')
    crates.add(crate)
    manifest = (crate / 'Cargo.toml').read_text()
    pending.extend(crate / path for path in re.findall(r'path\s*=\s*"([^"]+)"', manifest) if (crate / path).is_dir())
for crate in crates:
    target = stage / crate.name
    if target.exists():
        shutil.rmtree(target)
    shutil.copytree(crate, target)
for name in ('examples', 'library'):
    shutil.copytree(root / name, stage / name, dirs_exist_ok=True)
manifest = (root / 'Cargo.toml').read_text()
members = ', '.join('"' + crate.name + '"' for crate in sorted(crates))
manifest = manifest.replace('members = ["tla-*"]', 'members = [' + members + ']')
(stage / 'Cargo.toml').write_text(manifest)
shutil.copy2(root / 'Cargo.lock', stage / 'Cargo.lock')
print(f'Staged {len(crates)} crates in {stage}')
# Cargo removes irrelevant packages from the lockfile. Reject any resolution
# that changes a retained package rather than silently upgrading dependencies.
subprocess.run(['cargo', 'metadata', '--manifest-path', str(stage / 'Cargo.toml'),
                '--format-version', '1'], stdout=subprocess.DEVNULL, check=True)
def packages(path):
    return {block.strip() for block in path.read_text().split('[[package]]')[1:]}
if not packages(stage / 'Cargo.lock') <= packages(root / 'Cargo.lock'):
    raise RuntimeError('Staged dependency resolution changed a locked package')
