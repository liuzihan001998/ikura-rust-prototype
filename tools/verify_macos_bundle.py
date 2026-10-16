#!/usr/bin/env python3
"""Check relocation, signatures, media tools and SDL's dynamically loaded codecs."""
import argparse
import ctypes
import os
from pathlib import Path
import plistlib
import subprocess
import tempfile
from package_macos import dependencies, install_id, SYSTEM_PREFIXES, run


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("app", type=Path)
    args = parser.parse_args()
    app = args.app.resolve()
    contents = app / "Contents"
    info = plistlib.loads((contents / "Info.plist").read_bytes())
    binaries = list((contents / "Frameworks").glob("*.dylib"))
    binaries += list((contents / "MacOS").iterdir())
    binaries += list((contents / "Resources/tools").iterdir())
    seen = set()
    for binary in binaries:
        if binary.resolve() in seen:
            continue
        seen.add(binary.resolve())
        if run("lipo", "-archs", binary, capture=True) != "arm64":
            raise RuntimeError(f"Not an arm64-only Mach-O file: {binary}")
        own_id = install_id(binary)
        for dependency in dependencies(binary):
            if dependency == own_id or dependency.startswith(SYSTEM_PREFIXES):
                continue
            if not dependency.startswith("@loader_path/"):
                raise RuntimeError(f"Unrelocated dependency in {binary}: {dependency}")
            target = Path(dependency.replace("@loader_path", str(binary.parent))).resolve()
            if not target.is_relative_to(app) or not target.is_file():
                raise RuntimeError(f"Dependency missing/outside bundle: {target}")
    run("codesign", "--verify", "--deep", "--strict", app)
    environment = os.environ.copy()
    environment["PATH"] = "/usr/bin:/bin:/usr/sbin:/sbin"
    environment["SDL_AUDIODRIVER"] = "dummy"
    for variable in ("DYLD_LIBRARY_PATH", "DYLD_FALLBACK_LIBRARY_PATH", "DYLD_INSERT_LIBRARIES", "SDL_DYNAMIC_API"):
        environment.pop(variable, None)
    media = contents / "Resources/tools"
    with tempfile.TemporaryDirectory(prefix="ikura-media-check-") as directory:
        wave = Path(directory) / "test.wav"
        run(media / "ffmpeg", "-nostdin", "-v", "error", "-f", "lavfi", "-i",
            "sine=frequency=440:duration=0.1", "-ac", "2", "-ar", "48000", wave, env=environment)
        duration = run(media / "ffprobe", "-v", "error", "-show_entries", "format=duration",
                       "-of", "default=noprint_wrappers=1:nokey=1", wave, capture=True, env=environment)
        if not 0.09 <= float(duration) <= 0.11:
            raise RuntimeError(f"Unexpected decoded audio duration: {duration}")
    # Run the startup front-end without opening a chooser; an absent scripts
    # file is an expected ordinary error, while dyld failures must never pass.
    with tempfile.TemporaryDirectory(prefix="ikura-no-data-") as directory:
        launch_environment = environment.copy()
        launch_environment["IKURA_SAVE_DIR"] = str(Path(directory) / "saves")
        process = subprocess.run([str(contents / "MacOS/IkuraLauncher"), directory],
                                 env=launch_environment, text=True, capture_output=True)
        if process.returncode != 1 or "Ikura failed:" not in process.stderr:
            raise RuntimeError(f"Launcher/engine did not produce a normal data error: {process.stderr}")
    # A child process avoids polluting the verifier's Python with SDL and tests
    # the same @loader_path SDL3 fallback used by sdl2-compat on the user's Mac.
    probe = r'''
import ctypes, pathlib, sys
root=pathlib.Path(sys.argv[1])
sdl=ctypes.CDLL(str(root / "libSDL2-2.0.0.dylib"))
sdl.SDL_Init.argtypes=[ctypes.c_uint32]
sdl.SDL_GetError.restype=ctypes.c_char_p
if sdl.SDL_Init(0x10)!=0: raise RuntimeError(sdl.SDL_GetError())
try:
    mixer=ctypes.CDLL(str(root / "libSDL2_mixer-2.0.0.dylib"))
    mixer.Mix_Init.argtypes=[ctypes.c_int]
    if mixer.Mix_Init(0x10)&0x10==0: raise RuntimeError(sdl.SDL_GetError())
    mixer.Mix_Quit()
finally: sdl.SDL_Quit()
'''
    run("/usr/bin/python3", "-c", probe, contents / "Frameworks", env=environment)
    print(f"Verified {len(seen)} arm64 binaries; signatures, relocations, SDL/OGG, FFmpeg and launcher OK.")
    print(f"Declared minimum macOS: {info['LSMinimumSystemVersion']}")


if __name__ == "__main__":
    main()
