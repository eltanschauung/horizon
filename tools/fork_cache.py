"""Bound generated resource snapshots and known disposable workflow outputs."""
from contextlib import ExitStack
import os
from pathlib import Path
import re
import shutil
import time


_RELEASE_VERSION = r'v[0-9]+(?:\.[0-9]+){2}'
_NATIVE_VERSION = r'native-[0-9a-f]+-[0-9a-f]+'
_PLAYER_DATA = ('saves', 'world', 'playerdata', 'options.txt', 'server.toml', 'server.properties', '.git')


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
    # Both pipelines need their current snapshot; a combined limit evicts one
    # pipeline whenever the other runs, forcing the next launch to regenerate it.
    seen = set()
    kept, candidates = [], []
    for _, _, p in snapshots:
        if p.parent.name in seen:
            candidates.append(p)
        else:
            kept.append(str(p))
            seen.add(p.parent.name)
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
    workflow = purge_workflow_cache()
    launcher = purge_launcher_cache(keep_versions=workflow.get('base_versions', ()))
    if workflow['removed'] or workflow.get('deferred'):
        print(json.dumps({'workflow_cache': workflow}), flush=True)
    if launcher['removed'] or launcher.get('deferred'):
        print(json.dumps({'launcher_cache': launcher}), flush=True)
    if report['removed'] or report.get('deferred'):
        print(json.dumps({'resource_cache': report}), flush=True)


def purge_launcher_cache(home=None, *, apply=True, workflow_locked=False, keep_versions=(), interrupted_only=False):
    """Retain the newest snapshot per family, plus live/referenced builds."""
    from fork_workflow import build_home, homes, read, running_paths
    root = Path(home) if home is not None else homes()[0]
    if apply and not workflow_locked:
        import fcntl
        root.mkdir(parents=True, exist_ok=True)
        with (root/'.fork-workflow.lock').open('a') as lock:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                return {'deferred': 'Build/deployment workflow is running', 'removed': []}
            return purge_launcher_cache(root, apply=True, workflow_locked=True, keep_versions=keep_versions, interrupted_only=interrupted_only)
    native = build_home(root)/'fork-native'
    if native.is_symlink():
        return {'deferred': 'Native build directory is a symlink', 'removed': []}
    # Avoid racing a direct native build as well as workflow-managed builds.
    if apply:
        if _native_compiler_running(native):
            return {'deferred': 'Native compiler is running', 'removed': []}
    refs = {read(root/'settings.json').get('selected'), *keep_versions}
    refs.update(v.get('version') for v in read(root/'instances.json').get('instances', {}).values())
    versions = root/'fork-versions'
    server_builds = native/'server-builds'
    # A native build can still use its original installed release as a build base.
    for parent in (versions, server_builds):
        if parent.is_dir() and not parent.is_symlink():
            for p in parent.iterdir():
                metadata = p/'native-mods.json'
                if re.fullmatch(_NATIVE_VERSION, p.name) and not p.is_symlink() and not metadata.is_symlink():
                    refs.add(read(metadata).get('base'))
    revisions = {v.get('horizon') for v in read(native/'built.json').values() if isinstance(v, dict)}
    active = set(running_paths())
    families = [('server-builds', server_builds, _NATIVE_VERSION),
                ('source', native/'source', r'[0-9a-f]{40}'),
                ('patched', native/'patched', r'[0-9a-f]{64}'),
                ('fork-versions', versions, _NATIVE_VERSION),
                ('release-versions', versions, _RELEASE_VERSION),
                ('server-partial', server_builds, rf'\.{_NATIVE_VERSION}\.partial'),
                ('client-partial', versions, rf'\.{_NATIVE_VERSION}\.partial'),
                ('source-partial', native/'source', r'\.[0-9a-f]{40}\.partial'),
                ('patched-partial', native/'patched', r'\.[0-9a-f]{64}\.partial'),
                ('staging-partial', native, r'stage\.new|(?:installed|mods|mixins)\.incoming')]
    cutoff = time.time() - 300
    report = {'families': {}, 'removed': []}
    for name, parent, pattern in families:
        partial = name.endswith('-partial')
        if (interrupted_only and not partial) or parent.is_symlink() or not parent.is_dir():
            continue
        entries = [p for p in parent.iterdir() if not p.is_symlink() and p.is_dir()
                   and re.fullmatch(pattern, p.name)]
        if name == 'release-versions':
            entries = [p for p in entries if (p/'.installed').is_file() and not (p/'.installed').is_symlink()]
            entries.sort(key=lambda p: tuple(map(int, p.name[1:].split('.'))), reverse=True)
        else:
            entries.sort(key=lambda p: (p.stat().st_mtime_ns, p.name), reverse=True)
        kept, candidates = [], []
        for i, p in enumerate(entries):
            resolved = p.resolve()
            protected = ((partial and p.stat().st_mtime >= cutoff)
                         or (parent == versions and p.name in refs) or (name == 'source' and p.name in revisions)
                         or any(a.is_relative_to(resolved) for a in active)
                         or any((p/n).exists() or (p/n).is_symlink() for n in _PLAYER_DATA))
            (kept if (i == 0 and not partial) or protected else candidates).append(p)
        report['families'][name] = {'kept': [str(p) for p in kept],
                                    'candidates': [str(p) for p in candidates]}
        if apply:
            for p in candidates:
                shutil.rmtree(p)
                report['removed'].append(str(p))
    if interrupted_only:
        return report
    windows = _purge_retired_windows_output(native/'target-windows', active, apply=apply)
    report['families']['windows-cross-build'] = windows
    report['removed'].extend(windows['removed'])
    return report


def _purge_retired_windows_output(cache, active, *, apply):
    """Clear the old portable-build scratch profiles when idle for three days."""
    import fcntl

    report = {'kept': [], 'candidates': [], 'removed': []}
    tag = cache/'CACHEDIR.TAG'
    if cache.is_symlink() or tag.is_symlink() or not tag.is_file():
        return report
    with tag.open('rb') as stream:
        if stream.readline(100).strip() != b'Signature: 8a477f597d28d172789f06886806bc55':
            return report
    platform = cache/'x86_64-pc-windows-gnu'
    if platform.is_symlink() or not platform.is_dir():
        return report
    profiles = (cache/'fast', platform/'fast')
    report['kept'] = [str(cache)]
    if any(a.is_relative_to(cache.resolve()) for a in active):
        return report
    lock_names = {'.cargo-lock', '.cargo-build-lock', '.cargo-artifact-lock'}
    cutoff = time.time() - 3 * 24 * 60 * 60
    with ExitStack() as locks:
        for profile in profiles:
            if profile.is_symlink() or not profile.is_dir():
                return report
            lock_paths = [p for p in profile.iterdir() if p.name in lock_names]
            if not lock_paths or any(p.is_symlink() or not p.is_file() for p in lock_paths):
                return report
            for path in lock_paths:
                try:
                    lock = locks.enter_context(path.open('rb'))
                    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                except (BlockingIOError, FileNotFoundError):
                    return report
        outputs = [p for profile in profiles for p in sorted(profile.iterdir()) if p.name not in lock_names]
        for path in outputs:
            if path.is_symlink() or any((path/n).exists() or (path/n).is_symlink() for n in _PLAYER_DATA):
                return report
            if path.name in {'.fingerprint', 'build', 'deps', 'examples', 'incremental'} and path.is_dir():
                entries = path.rglob('*')
            elif re.fullmatch(r'horizon-[\w-]+\.(?:exe|d)', path.name) and path.is_file():
                entries = ()
            else:
                return report
            if path.lstat().st_mtime >= cutoff or any(p.lstat().st_mtime >= cutoff for p in entries):
                return report
        report['candidates'] = [str(p) for p in outputs]
        if apply:
            # The old helper and Cargo can reuse these exact lock inodes on a later build.
            for path in outputs:
                if path.is_dir():
                    shutil.rmtree(path)
                else:
                    path.unlink()
                report['removed'].append(str(path))
    return report


# Only generated children of validated frozen plans are owned by this cleaner.
# Metadata, handoffs, package backups, source trees and test worlds stay.
_WORKFLOW_OUTPUTS = ('inputs', 'native-input', 'server-export', 'server-export.previous', 'upload')


def purge_workflow_cache(repo=None, *, active_plan=None, apply=True, workflow_locked=False):
    """Keep newest successful plan per target and newest pending/active inputs.

    Deletes complete obsolete export copies, including their duplicated prepared
    resources. This does not touch the shared SDK or compiler/dependency caches.
    A failed replacement retains the last verified export for recovery.
    """
    import hashlib
    import json
    from fork_workflow import homes, running_paths
    repo = Path(repo) if repo is not None else Path(__file__).resolve().parents[1]
    launcher = homes()[0]
    if apply and not workflow_locked:
        import fcntl
        launcher.mkdir(parents=True, exist_ok=True)
        with (launcher/'.fork-workflow.lock').open('a') as lock:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                return {'deferred': 'Build/deployment workflow is running', 'removed': []}
            return purge_workflow_cache(repo, active_plan=active_plan, workflow_locked=True)
    if apply and _native_compiler_running(launcher/'fork-native'):
        return {'deferred': 'Native compiler is running', 'removed': []}

    def read_json(path):
        try:
            return json.loads(path.read_text())
        except (OSError, ValueError):
            return {}

    active = set(running_paths())
    active_plan = Path(active_plan).resolve().parent if active_plan is not None else None
    plans = []
    # Older agents placed plans in all three repositories and at varying depths.
    for checkout in (repo, repo.parent/'Lunatic', repo.parent/'horizon-mods'):
        base = checkout/'.local'
        if base.is_symlink() or not base.is_dir():
            continue
        for directory, dirs, files in os.walk(base, followlinks=False):
            path = Path(directory)
            dirs[:] = [d for d in dirs if d not in {
                'target', '.git', 'node_modules', 'assets', 'resources', 'crates',
                'tools', 'saves', 'world', 'playerdata', '.horizon-cache', *_WORKFLOW_OUTPUTS
            } and not (path/d).is_symlink()]
            if (path/'.git').exists() or len(path.relative_to(base).parts) >= 8:
                dirs[:] = []
                continue
            if 'manifest.json' not in files or (path/'manifest.json').is_symlink():
                continue
            m = read_json(path/'manifest.json')
            if not (isinstance(m, dict) and m.get('schema') == 1
                    and re.fullmatch('[0-9a-f]{40}', str(m.get('game_revision', '')))
                    and isinstance(m.get('packages'), list) and m['packages']):
                continue
            # Recognize only the workflow's fixed input layout. No manifest
            # path can grant permission to delete an arbitrary directory.
            if not all(isinstance(e, dict) and isinstance(e.get('path'), str)
                       and len(Path(e['path']).parts) == 3
                       and Path(e['path']).parts[0] == 'inputs'
                       and Path(e['path']).parts[1] in ('horizon-mods', 'horizon-native-mods')
                       and '..' not in Path(e['path']).parts for e in m['packages']):
                continue
            a = read_json(path/'artifacts.json')
            valid = isinstance(a, dict) and a.get('manifest_sha256') == hashlib.sha256((path/'manifest.json').read_bytes()).hexdigest()
            targets = {t for t in ('client', 'server') if valid and isinstance(a.get(t), dict) and a[t].get('files')}
            completed = (path/'artifacts.json').stat().st_mtime_ns if targets else 0
            plans.append((path, (path/'manifest.json').stat().st_mtime_ns, completed, targets, a))
    kept = {active_plan} if active_plan else set()
    for target in ('client', 'server'):
        successes = [p for p in plans if target in p[3]]
        if successes:
            kept.add(max(successes, key=lambda p: (p[2], str(p[0])))[0])
    last_success = max((p[2] for p in plans), default=0)
    # Retain just the newest pending snapshot, plus an explicit active plan.
    # Failed preflight iterations must not accumulate a launcher copy each.
    # Preflight itself never promotes or purges a plan.
    pending = [p for p in plans if not p[3] and p[1] >= last_success]
    if pending:
        kept.add(max(pending, key=lambda p: (p[1], str(p[0])))[0])

    candidates = []
    protected = []
    def consider(path):
        if path.is_symlink() or not path.is_dir():
            return
        resolved = path.resolve()
        if (any(a.is_relative_to(resolved) for a in active)
                or any((path/n).exists() or (path/n).is_symlink() for n in _PLAYER_DATA)):
            protected.append(str(path))
            return
        candidates.append(path)

    for path, _, _, targets, artifacts in plans:
        if path not in kept:
            # Protect the whole plan if an executable/working directory is in
            # use, even if it happens to be in a different generated child.
            resolved = path.resolve()
            if any(a.is_relative_to(resolved) for a in active):
                protected.append(str(path))
                continue
            for name in _WORKFLOW_OUTPUTS:
                consider(path/name)
        elif 'server' in targets and (path/'server-export.previous').is_dir():
            # Only drop the previous export after the replacement is verified;
            # stale artifacts.json from a failed attempt is not sufficient.
            server = artifacts['server']
            current = path/'server-export'
            files = server['files']
            if (server.get('directory') == str(current) and isinstance(files, dict)
                    and files and all(Path(n).name == n for n in files)):
                try:
                    def matches(name, expected):
                        with (current/name).open('rb') as stream:
                            return hashlib.file_digest(stream, 'sha256').hexdigest() == expected
                    verified = all(matches(n, sha) for n, sha in files.items())
                except OSError:
                    verified = False
                if verified:
                    consider(path/'server-export.previous')
    bases = (read_json(path/'manifest.json').get('base') for path in kept)
    report = {'kept': sorted(str(p) for p in kept), 'protected': protected,
              'base_versions': sorted({b for b in bases if isinstance(b, str)}),
              'candidates': [str(p) for p in candidates], 'removed': []}
    if apply:
        for path in candidates:
            shutil.rmtree(path)
            report['removed'].append(str(path))
    return report


def _native_compiler_running(native):
    for proc in Path('/proc').glob('[0-9]*'):
        try:
            name = (proc/'comm').read_text().strip()
            args = (proc/'cmdline').read_bytes().replace(b'\0', b' ').decode(errors='replace')
            if (name in {'cargo', 'rustc', 'horizon-mixins'} and str(native) in args
                    or name.startswith('horizon-launch') and 'native-build' in args):
                return True
        except OSError:
            pass
    return False
