"""Bound generated game resource snapshots, never compiler caches or player data."""
import os
from pathlib import Path
import re
import shutil


def game_home():
    data = Path(os.environ.get('XDG_DATA_HOME', str(Path.home()/'.local/share')))
    return Path(os.environ.get('HORIZON_HOME', str(data/'horizon')))


def game_running():
    for proc in Path('/proc').glob('[0-9]*'):
        try:
            if (proc/'exe').resolve(strict=True).name in {'horizon-client', 'horizon-engine'}:
                return True
        except OSError:
            pass
    return False


def purge_resource_cache(home=None, *, apply=True):
    cache = (Path(home) if home is not None else game_home())/'.horizon-cache'
    snapshots = []
    for family in ('mod-content', 'resourcepacks'):
        parent = cache/family
        if parent.is_symlink() or not parent.is_dir():
            continue
        for path in parent.iterdir():
            if path.is_symlink() or not path.is_dir() or not re.fullmatch('[0-9a-f]{16}', path.name):
                continue
            # The game touches this stamp on reuse; otherwise use folder mtime.
            stamp = path/'horizon-mod-content.stamp'
            recent = max(path.stat().st_mtime_ns, stamp.stat().st_mtime_ns if stamp.is_file() else 0)
            snapshots.append((recent, str(path), path))
    snapshots.sort(reverse=True)
    kept = [str(p) for _, _, p in snapshots[:3]]
    candidates = [p for _, _, p in snapshots[3:]]
    result = {'kept': kept, 'candidates': [str(p) for p in candidates], 'removed': []}
    if not apply:
        return result
    if game_running():
        result['deferred'] = 'Horizon is running'
        return result
    for path in candidates:
        shutil.rmtree(path)
        # Resource overlay metadata sits beside its generated folder.
        for suffix in ('.overlay-cache.json', '.overlay-cache.lock'):
            sidecar = path.with_name(path.name+suffix)
            if not sidecar.is_symlink():
                sidecar.unlink(missing_ok=True)
        result['removed'].append(str(path))
    return result


def cleanup_after_build():
    import json
    report = purge_resource_cache()
    if report['removed'] or report.get('deferred'):
        print(json.dumps({'resource_cache': report}), flush=True)
