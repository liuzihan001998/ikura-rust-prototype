use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

fn run(command: &mut Command, description: &str) {
    let status = command
        .status()
        .unwrap_or_else(|error| panic!("could not {description}: {error}"));
    assert!(
        status.success(),
        "failed to {description} (exit status {status})"
    );
}

fn android_sdl2_prefix(build_dir: &Path) -> PathBuf {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let entries = fs::read_dir(build_dir)
            .unwrap_or_else(|error| panic!("could not inspect SDL build output: {error}"));
        for entry in entries.flatten() {
            let path = entry.path().join("out");
            if entry.file_name().to_string_lossy().starts_with("sdl2-sys-")
                && path.join("lib/cmake/SDL2/SDL2Config.cmake").is_file()
            {
                return path;
            }
        }
        if Instant::now() >= deadline {
            break;
        }
        thread::sleep(Duration::from_millis(250));
    }
    panic!("sdl2-sys did not provide an SDL2 CMake package");
}

fn main() {
    println!("cargo:rerun-if-changed=android/native/SDL2_mixer/CMakeLists.txt");
    println!("cargo:rerun-if-changed=android/native/SDL2_mixer/src");
    println!("cargo:rerun-if-changed=android/cmake/ikura-android.toolchain.cmake");
    println!("cargo:rerun-if-env-changed=ANDROID_NDK_HOME");
    println!("cargo:rerun-if-env-changed=SDL2_TOOLCHAIN");
    println!("cargo:rerun-if-env-changed=CMAKE");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("android") {
        return;
    }

    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let target_build_dir = out_dir.parent().unwrap().parent().unwrap();
    let sdl2_prefix = android_sdl2_prefix(target_build_dir);
    let build_dir = out_dir.join("sdl2-mixer-build");
    let install_dir = out_dir.join("sdl2-mixer-install");
    let cmake = env::var_os("CMAKE").unwrap_or_else(|| "cmake".into());

    run(
        Command::new(&cmake)
            .arg("-S")
            .arg(manifest.join("android/native/SDL2_mixer"))
            .arg("-B")
            .arg(&build_dir)
            .arg(format!(
                "-DCMAKE_TOOLCHAIN_FILE={}",
                env::var("SDL2_TOOLCHAIN").unwrap_or_else(|_| {
                    manifest
                        .join("android/cmake/ikura-android.toolchain.cmake")
                        .display()
                        .to_string()
                })
            ))
            .arg(format!(
                "-DSDL2_DIR={}",
                sdl2_prefix.join("lib/cmake/SDL2").display()
            ))
            .arg(format!("-DCMAKE_INSTALL_PREFIX={}", install_dir.display()))
            .args([
                "-DCMAKE_BUILD_TYPE=Release",
                "-DSDL2MIXER_SAMPLES=OFF",
                "-DSDL2MIXER_VENDORED=OFF",
                "-DSDL2MIXER_FLAC=OFF",
                "-DSDL2MIXER_GME=OFF",
                "-DSDL2MIXER_MOD=OFF",
                "-DSDL2MIXER_MP3=OFF",
                "-DSDL2MIXER_MIDI=OFF",
                "-DSDL2MIXER_OPUS=OFF",
                "-DSDL2MIXER_VORBIS=STB",
                "-DSDL2MIXER_WAVPACK=OFF",
            ]),
        "configure SDL2_mixer",
    );
    run(
        Command::new(&cmake)
            .arg("--build")
            .arg(&build_dir)
            .arg("--parallel")
            .arg(env::var("CARGO_BUILD_JOBS").unwrap_or_else(|_| "4".into())),
        "build SDL2_mixer",
    );
    run(
        Command::new(&cmake).arg("--install").arg(&build_dir),
        "install SDL2_mixer",
    );

    println!(
        "cargo:rustc-link-search=native={}",
        install_dir.join("lib").display()
    );
    println!("cargo:rustc-link-lib=dylib=SDL2_mixer");
}
