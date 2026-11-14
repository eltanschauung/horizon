#!/usr/bin/env python3
"""Fetch the pinned, native x64 Opus runtime for Windows release/portable packaging."""
import argparse,hashlib,io,tarfile,urllib.request
from pathlib import Path
URL='https://mirror.msys2.org/mingw/mingw64/mingw-w64-x86_64-opus-1.6.1-2-any.pkg.tar.zst'
SHA256='fae981c518ed1ff2c5264eb5a0d590ec1d2ebb222defb2253ca85b2154fd8c04'
def prepare(destination, cache):
    destination.mkdir(parents=True,exist_ok=True);cache.mkdir(parents=True,exist_ok=True)
    archive=cache/URL.rsplit('/',1)[1]
    if not archive.exists():
        with urllib.request.urlopen(URL,timeout=90) as response:
            data=response.read(5_000_000)
        if hashlib.sha256(data).hexdigest()!=SHA256:raise ValueError('Opus archive checksum mismatch')
        archive.write_bytes(data)
    data=archive.read_bytes()
    if hashlib.sha256(data).hexdigest()!=SHA256:raise ValueError('Cached Opus checksum mismatch')
    try:
        from compression import zstd
        raw=zstd.decompress(data)
    except ImportError:
        import zstandard
        raw=zstandard.ZstdDecompressor().stream_reader(io.BytesIO(data)).read()
    with tarfile.open(fileobj=io.BytesIO(raw)) as t:
        for source,target in [('mingw64/bin/libopus-0.dll','libopus-0.dll'),('mingw64/share/licenses/opus/COPYING','OPUS-LICENSE.txt')]:
            member=t.getmember(source)
            if not member.isfile():raise ValueError('Invalid Opus member')
            content=t.extractfile(member).read()
            if target.endswith('.dll') and not content.startswith(b'MZ'):raise ValueError('Invalid Opus DLL')
            (destination/target).write_bytes(content)
    print('Prepared verified x64 Opus DLL and license')
if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--output',type=Path,required=True);p.add_argument('--cache',type=Path,default=Path.home()/'.cache/horizon-windows-runtime');a=p.parse_args();prepare(a.output,a.cache)
