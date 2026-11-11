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
    launcher = purge_launcher_cache()
    if launcher['removed'] or launcher.get('deferred'):
        print(json.dumps({'launcher_cache': launcher}), flush=True)
    if report['removed'] or report.get('deferred'):
        print(json.dumps({'resource_cache': report}), flush=True)


def purge_launcher_cache(home=None, *, apply=True, workflow_locked=False):
    """Retain two reproducible snapshots per family, plus live/referenced builds."""
    import json
    from fork_workflow import homes, read, running_paths
    root = Path(home) if home is not None else homes()[0]
    if apply and not workflow_locked:
        import fcntl
        root.mkdir(parents=True, exist_ok=True)
        with (root/'.fork-workflow.lock').open('a') as lock:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                return {'deferred': 'Build/deployment workflow is running', 'removed': []}
            return purge_launcher_cache(root, apply=True, workflow_locked=True)
    native = root/'fork-native'
    # Avoid racing a direct native build as well as workflow-managed builds.
    if apply:
        for proc in Path('/proc').glob('[0-9]*'):
            try:
                name = (proc/'comm').read_text().strip()
                args = (proc/'cmdline').read_bytes().replace(b'\0', b' ').decode(errors='replace')
                if (name in {'cargo', 'rustc'} and str(native) in args
                        or name.startswith('horizon-launch') and 'native-build' in args):
                    return {'deferred': 'Native compiler is running', 'removed': []}
            except OSError:
                pass
    refs = {read(root/'settings.json').get('selected')}
    refs.update(v.get('version') for v in read(root/'instances.json').get('instances', {}).values())
    revisions = {v.get('horizon') for v in read(native/'built.json').values() if isinstance(v, dict)}
    active = set(running_paths())
    families = [('server-builds', native/'server-builds', r'native-[0-9a-f]+-[0-9a-f]+'),
                ('source', native/'source', r'[0-9a-f]{40}'),
                ('patched', native/'patched', r'[0-9a-f]{64}'),
                ('fork-versions', root/'fork-versions', r'native-[0-9a-f]+-[0-9a-f]+')]
    report = {'families': {}, 'removed': []}
    for name, parent, pattern in families:
        if parent.is_symlink() or not parent.is_dir():
            continue
        entries = sorted((p for p in parent.iterdir() if not p.is_symlink() and p.is_dir()
                          and re.fullmatch(pattern, p.name)),
                         key=lambda p: (p.stat().st_mtime_ns, p.name), reverse=True)
        kept, candidates = [], []
        for i, p in enumerate(entries):
            resolved = p.resolve()
            protected = ((name == 'fork-versions' and p.name in refs) or (name == 'source' and p.name in revisions)
                         or any(a.is_relative_to(resolved) for a in active)
                         or any((p/n).exists() for n in ('saves', 'world', 'playerdata', 'options.txt', '.git')))
            (kept if i < 2 or protected else candidates).append(p)
        report['families'][name] = {'kept': [str(p) for p in kept],
                                    'candidates': [str(p) for p in candidates]}
        if apply:
            for p in candidates:
                shutil.rmtree(p)
                report['removed'].append(str(p))
    return report
