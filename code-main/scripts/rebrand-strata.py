from pathlib import Path
import subprocess

root = Path(__file__).resolve().parents[1]
subprocess.run(['node', str(root / 'scripts' / 'build-strata-brand.mjs')], cwd=root, check=True)
