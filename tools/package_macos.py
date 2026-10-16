#!/usr/bin/env python3
"""Build an arm64 .app and self-contained ZIP/DMG for a macOS GitHub Release."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent
SYSTEM_PREFIXES = ("/System/Library/", "/usr/lib/")


def run(*args, capture=False, **kwargs):
    result = subprocess.run([str(arg) for arg in args], check=True,
                            text=True, stdout=subprocess.PIPE if capture else None, **kwargs)
    return result.stdout.strip() if capture else None


def dependencies(binary):
    # Mach-O install names can contain spaces. Do not split them on whitespace.
    return [line.strip().split(" (", 1)[0]
            for line in run("otool", "-L", binary, capture=True).splitlines()[1:]
            if " (" in line]


def install_id(binary):
    lines = run("otool", "-D", binary, capture=True).splitlines()
    return lines[1].strip() if len(lines) > 1 else None


def rpaths(binary):
    return re.findall(r"cmd LC_RPATH\n.*?\n\s*path (.*?) \(offset",
                      run("otool", "-l", binary, capture=True))


def deployment_version(binary):
    output = run("otool", "-l", binary, capture=True)
    found = re.findall(r"\bminos (\d+(?:\.\d+)+)", output)
    found += re.findall(r"cmd LC_VERSION_MIN_MACOSX\n.*?\n\s*version (\d+(?:\.\d+)+)", output)
    return max((tuple(map(int, value.split("."))) for value in found), default=(11, 0))


def expand_loader(value, binary, executable):
    return value.replace("@loader_path", str(binary.parent)).replace(
        "@executable_path", str(executable.parent))


def resolve_dependency(value, binary, executable):
    if value.startswith("@rpath/"):
        suffix = value[len("@rpath/"):]
        candidates = [Path(expand_loader(p, binary, executable)) / suffix for p in rpaths(binary)]
        candidates += [Path(expand_loader(p, executable, executable)) / suffix
                       for p in rpaths(executable)]
        candidates.append(binary.parent / suffix)
    else:
        candidates = [Path(expand_loader(value, binary, executable))]
    for candidate in candidates:
        if candidate.is_file():
            return candidate.resolve()
    raise RuntimeError(f"Cannot resolve {value} loaded by {binary}")


class Bundle:
    def __init__(self, app):
        self.app = app
        self.frameworks = app / "Contents/Frameworks"
        self.frameworks.mkdir(parents=True)
        self.files = {}  # canonical source -> bundled destination
        self.edges = {}
        self.minimum = (14, 0)

    def copy(self, source, destination):
        source = source.resolve()
        if source in self.files:
            return
        arches = run("lipo", "-archs", source, capture=True).split()
        if "arm64" not in arches:
            raise RuntimeError(f"arm64 slice missing: {source} ({arches})")
        destination.parent.mkdir(parents=True, exist_ok=True)
        if destination.exists():
            raise RuntimeError(f"Dependency basename collision: {destination}")
        if len(arches) > 1:
            run("lipo", source, "-thin", "arm64", "-output", destination)
        else:
            shutil.copyfile(source, destination)
        destination.chmod(0o755)
        subprocess.run(["codesign", "--remove-signature", str(destination)],
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
        self.files[source] = destination
        self.minimum = max(self.minimum, deployment_version(source))

    def collect(self, source, destination, executable=None):
        source = source.resolve()
        if source in self.edges:
            return
        self.copy(source, destination)
        self.edges[source] = []
        own_id = install_id(source)
        for value in dependencies(source):
            if value == own_id or value.startswith(SYSTEM_PREFIXES):
                continue
            dependency = resolve_dependency(value, source, executable or source)
            self.edges[source].append((value, dependency))
            self.collect(dependency, self.frameworks / dependency.name, executable or source)
        # Homebrew sdl2-compat loads SDL3 with dlopen, so otool cannot discover it.
        if "sdl2-compat" in source.parts:
            prefix = Path(run("brew", "--prefix", "sdl3", capture=True))
            sdl3 = (prefix / "lib/libSDL3.dylib").resolve()
            self.collect(sdl3, self.frameworks / sdl3.name, executable or source)
            alias = self.frameworks / "libSDL3.dylib"
            if alias.name != sdl3.name and not alias.exists():
                alias.symlink_to(sdl3.name)

    def relocate(self):
        for source, destination in self.files.items():
            if destination.parent == self.frameworks:
                run("install_name_tool", "-id", "@rpath/" + destination.name, destination)
            for old, dependency in self.edges[source]:
                target = self.files[dependency]
                relative = os.path.relpath(target, destination.parent)
                run("install_name_tool", "-change", old, "@loader_path/" + relative, destination)
            for value in set(rpaths(destination)):
                if value.startswith("/") and not value.startswith(SYSTEM_PREFIXES):
                    run("install_name_tool", "-delete_rpath", value, destination)

    def verify(self):
        for destination in self.files.values():
            own_id = install_id(destination)
            for value in dependencies(destination):
                if value == own_id or value.startswith(SYSTEM_PREFIXES):
                    continue
                if not value.startswith("@loader_path/"):
                    raise RuntimeError(f"External dependency remains: {destination}: {value}")
                target = Path(value.replace("@loader_path", str(destination.parent))).resolve()
                if not target.is_relative_to(self.app.resolve()) or not target.is_file():
                    raise RuntimeError(f"Dependency escapes bundle or is missing: {target}")

    def notices(self, resources):
        notices = resources / "ThirdParty"
        notices.mkdir()
        installed = json.loads(run("brew", "info", "--json=v2", "--installed", capture=True))
        formulas = {entry["name"]: entry for entry in installed["formulae"]}
        records = {}
        for source in self.files:
            parts = source.parts
            if "Cellar" not in parts:
                continue
            position = parts.index("Cellar")
            name, version = parts[position + 1:position + 3]
            key = f"{name}-{version}"
            if key in records:
                continue
            prefix = Path(*parts[:position + 3])
            formula = formulas.get(name, {})
            records[key] = {"name": name, "version": version,
                            "license": formula.get("license"), "homepage": formula.get("homepage"),
                            "source": formula.get("urls", {}).get("stable", {})}
            target = notices / key
            target.mkdir()
            for pattern in ("LICENSE*", "COPYING*", "NOTICE*", "share/doc/*/LICENSE*",
                            "share/doc/*/COPYING*", ".brew/*.rb"):
                for path in prefix.glob(pattern):
                    if path.is_file():
                        shutil.copyfile(path, target / path.name)
        (resources / "THIRD_PARTY.json").write_text(
            json.dumps(list(records.values()), ensure_ascii=False, indent=2) + "\n")
        metadata = json.loads(run("cargo", "metadata", "--locked", "--format-version=1",
                                  capture=True, cwd=ROOT))
        rust = [{key: package.get(key) for key in ("name", "version", "license", "repository", "source")}
                for package in metadata["packages"] if package["source"]]
        (notices / "RUST_DEPENDENCIES.json").write_text(json.dumps(rust, indent=2) + "\n")
        for package in metadata["packages"]:
            if not package["source"]:
                continue
            crate = Path(package["manifest_path"]).parent
            destination = notices / "Rust" / f"{package['name']}-{package['version']}"
            for pattern in ("LICENSE*", "COPYING*", "NOTICE*", "license*"):
                for path in crate.glob(pattern):
                    if path.is_file():
                        destination.mkdir(parents=True, exist_ok=True)
                        shutil.copyfile(path, destination / path.name)
        (notices / "FFmpeg-license.txt").write_text(run("ffmpeg", "-L", capture=True))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", default="0.1.0-alpha.1")
    parser.add_argument("--output", type=Path, default=ROOT / "dist")
    parser.add_argument("--skip-build", action="store_true", help="Use the existing local Release executable")
    parser.add_argument("--no-dmg", action="store_true")
    args = parser.parse_args()
    if not re.fullmatch(r"\d+\.\d+\.\d+(?:-[A-Za-z0-9.]+)?", args.version):
        parser.error("version must look like 0.1.0-alpha.1")
    if run("uname", "-s", capture=True) != "Darwin" or run("uname", "-m", capture=True) != "arm64":
        parser.error("Build this package on an Apple Silicon Mac")
    for tool in ("cargo", "swiftc", "ffmpeg", "ffprobe", "brew", "codesign", "install_name_tool", "lipo"):
        if not shutil.which(tool):
            parser.error(f"Required build tool missing: {tool}")
    package = re.search(r'(?m)^name\s*=\s*"([^"]+)"', (ROOT / "Cargo.toml").read_text()).group(1)
    if not args.skip_build:
        run("cargo", "build", "--release", "--locked", "--bin", package, cwd=ROOT)
    output = args.output.resolve() / f"macos-v{args.version}-arm64"
    output.mkdir(parents=True, exist_ok=True)
    name = f"Ikura-v{args.version}-macos-arm64"
    with tempfile.TemporaryDirectory(prefix=".stage-", dir=output) as temporary:
        stage = Path(temporary)
        app = stage / "Ikura.app"
        resources = app / "Contents/Resources"
        macos = app / "Contents/MacOS"
        resources.mkdir(parents=True)
        macos.mkdir()
        launcher = stage / "IkuraLauncher"
        run("swiftc", "-O", "-target", "arm64-apple-macos14.0", "-framework", "AppKit",
            "-Xlinker", "-headerpad_max_install_names", ROOT / "packaging/macos/Launcher.swift", "-o", launcher)
        bundle = Bundle(app)
        bundle.collect(ROOT / f"target/release/{package}", macos / "ikura-engine")
        bundle.collect(launcher, macos / "IkuraLauncher")
        for tool in ("ffmpeg", "ffprobe"):
            bundle.collect(Path(shutil.which(tool)), resources / "tools" / tool)
        bundle.relocate()
        bundle.verify()
        minimum = ".".join(map(str, bundle.minimum))
        info = {"CFBundleExecutable": "IkuraLauncher", "CFBundleIdentifier": "io.github.liuzihan200545.ikura",
                "CFBundleName": "Ikura", "CFBundleDisplayName": "Ikura",
                "CFBundlePackageType": "APPL", "CFBundleVersion": "1",
                "CFBundleShortVersionString": args.version.split("-", 1)[0],
                "LSMinimumSystemVersion": minimum, "NSHighResolutionCapable": True,
                "NSPrincipalClass": "NSApplication"}
        with (app / "Contents/Info.plist").open("wb") as stream:
            plistlib.dump(info, stream)
        bundle.notices(resources)
        source = run("git", "rev-parse", "HEAD", capture=True, cwd=ROOT)
        dirty = bool(run("git", "status", "--porcelain", capture=True, cwd=ROOT))
        notes = (f"# Ikura v{args.version} — Apple Silicon 测试版\n\n"
                 f"- 支持架构：arm64（M 系列）\n- 最低 macOS：{minimum}（由本次所有内置二进制的部署版本计算）\n"
                 f"- 源码提交：{source}{' + 本地未提交改动' if dirty else ''}\n"
                 "- 包含解释器、SDL 依赖、FFmpeg/FFprobe；不包含游戏数据或汉化文本。\n"
                 "- 双击启动后选择已准备的 game_data；按住 Option 启动可重新选择。\n"
                 "- 本次包为临时签名、未经过 Apple 公证的预发布测试版。\n\n"
                 "安装和操作说明见 README.zh-CN.md，已知效果精度限制见源码 README。\n")
        (resources / "RELEASE_NOTES.md").write_text(notes)
        readme = ROOT / "packaging/macos/README.zh-CN.md"
        shutil.copyfile(readme, resources / readme.name)
        # Sign after all install names and resources have reached their final form.
        # Signing CFBundleExecutable directly also validates nested code. Sign
        # the libraries/helpers first, then sign the main executable with its app.
        main_executable = macos / "IkuraLauncher"
        for destination in sorted(bundle.files.values(), key=lambda p: (p.parent != bundle.frameworks, str(p))):
            if destination != main_executable:
                run("codesign", "--force", "--sign", "-", destination)
        run("codesign", "--force", "--sign", "-", app)
        run("codesign", "--verify", "--deep", "--strict", app)
        final = output / "Ikura.app"
        if final.exists():
            shutil.rmtree(final)
        shutil.move(app, final)
        shutil.copyfile(readme, output / readme.name)
        (output / "RELEASE_NOTES.md").write_text(notes)
        # A staging volume keeps unrelated build outputs out of the DMG.
        volume = stage / "volume"
        volume.mkdir()
        (volume / "Ikura.app").symlink_to(final)
        shutil.copyfile(readme, volume / readme.name)
        (volume / "RELEASE_NOTES.md").write_text(notes)
        archive = output / f"{name}.zip"
        if archive.exists():
            archive.unlink()
        run("ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", final, archive)
        assets = [archive]
        if not args.no_dmg:
            # hdiutil preserves symlinks, so use a real application copy here.
            (volume / "Ikura.app").unlink()
            shutil.copytree(final, volume / "Ikura.app", symlinks=True)
            (volume / "Applications").symlink_to("/Applications")
            image = output / f"{name}.dmg"
            run("hdiutil", "create", "-ov", "-volname", f"Ikura {args.version}",
                "-srcfolder", volume, "-format", "UDZO", image)
            assets.append(image)
        sums = [hashlib.sha256(path.read_bytes()).hexdigest() + "  " + path.name for path in assets]
        (output / "SHA256SUMS").write_text("\n".join(sums) + "\n")
    print(f"\nRelease assets: {output}\nMinimum macOS: {minimum}\nBundled Mach-O files: {len(bundle.files)}")
    if bundle.minimum > (14, 0):
        print("NOTE: Local Homebrew dependencies require newer macOS. Use the macOS 14 GitHub workflow for its older build environment.")


if __name__ == "__main__":
    main()
