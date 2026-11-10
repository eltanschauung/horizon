#!/usr/bin/env python3
"""Regenerate region selection vectors using original TerraBlender Java layer classes."""
import argparse
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]

def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--bop', type=Path, default=Path.home()/'Desktop/horizon-lunatic/biomesoplenty-horizon')
    p.add_argument('--java-home', type=Path)
    p.add_argument('--update', action='store_true', help='Replace the checked-in numeric oracle vectors')
    a = p.parse_args()
    java = a.java_home or next((Path.home()/'.minecraft/runtime').glob('*/linux/*/bin/javac')).parent.parent
    tb = a.bop/'terrablender-reference/common/src/main/java/terrablender/worldgen/noise'
    work = ROOT/'.local/region-weight-oracle'
    classes = work/'classes'
    classes.mkdir(parents=True,exist_ok=True)
    cp = (a.bop/'tools/oracle/classpath.txt').read_text().strip()
    names = ['Area','AreaContext','ZoomLayer','AreaFactory','AreaTransformer0','AreaTransformer1','PixelTransformer']
    subprocess.run([java/'bin/javac','-cp',cp,'-d',classes,*[tb/(n+'.java') for n in names],
                    ROOT/'mods/horizon/src/test/java/dev/horizonlab/parity/RegionWeightOracle.java'],check=True)
    result = subprocess.check_output([java/'bin/java','-cp',str(classes)+':'+cp,
                                      'dev.horizonlab.parity.RegionWeightOracle'],text=True)
    fixture = ROOT/'crates/horizon-world/tests/fixtures/region_weights.csv'
    if a.update:
        fixture.write_text(result)
    else:
        assert result == fixture.read_text(), 'Java layer output differs from checked-in reference vectors'
    print(f'Java oracle: {len(result.splitlines())} matching region samples')

if __name__ == '__main__':
    main()
