use crate::{
    graphics::{Graphics, GraphicsSnapshot},
    vm::Vm,
};
use serde::{Deserialize, Serialize};
use std::{
    error::Error,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

const FORMAT_VERSION: u32 = 1;

#[derive(Default, Serialize, Deserialize)]
pub struct AudioState {
    pub music: Option<PathBuf>,
    pub voice: Option<PathBuf>,
}

#[derive(Serialize)]
struct SaveRef<'a> {
    version: u32,
    script_signature: u64,
    vm: &'a Vm,
    graphics: GraphicsSnapshot,
    audio: AudioState,
}

#[derive(Deserialize)]
struct SaveFile {
    version: u32,
    script_signature: u64,
    vm: Vm,
    graphics: GraphicsSnapshot,
    audio: AudioState,
}

pub fn script_signature(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

pub fn write(
    path: &Path,
    signature: u64,
    vm: &Vm,
    graphics: &Graphics,
    audio: AudioState,
) -> Result<(), Box<dyn Error>> {
    if !vm.can_save() {
        return Err("the script currently disables saving".into());
    }
    let state = SaveRef {
        version: FORMAT_VERSION,
        script_signature: signature,
        vm,
        graphics: graphics.snapshot()?,
        audio,
    };
    let parent = path.parent().ok_or("save path has no parent")?;
    fs::create_dir_all(parent)?;
    let temporary = path.with_extension("tmp");
    let result = (|| -> Result<(), Box<dyn Error>> {
        let mut file = fs::File::create(&temporary)?;
        serde_json::to_writer(&mut file, &state)?;
        file.flush()?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

pub fn read(
    path: &Path,
    signature: u64,
    vm: &mut Vm,
    graphics: &mut Graphics,
) -> Result<AudioState, Box<dyn Error>> {
    let state: SaveFile = serde_json::from_reader(fs::File::open(path)?)?;
    if state.version != FORMAT_VERSION {
        return Err("unsupported save format version".into());
    }
    if state.script_signature != signature {
        return Err("save belongs to a different script export".into());
    }
    for path in [&state.audio.music, &state.audio.voice]
        .into_iter()
        .flatten()
    {
        if !path.is_file() {
            return Err(format!("saved audio is missing: {}", path.display()).into());
        }
    }
    // Decode and validate all surfaces before changing either live component.
    let restored_graphics = graphics.restored(state.graphics)?;
    vm.restore(state.vm)?;
    *graphics = restored_graphics;
    Ok(state.audio)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::{Instruction, Script, Scripts};

    fn game() -> (Vm, Graphics) {
        let mut scripts = Scripts::new();
        scripts.insert(
            "TEST".into(),
            Script {
                code_start: 0,
                jumps: vec![],
                instructions: vec![
                    Instruction {
                        offset: 0,
                        opcode: 0xF5,
                        payload: vec![1],
                    },
                    Instruction {
                        offset: 1,
                        opcode: 0x02,
                        payload: b"CHILD\0".to_vec(),
                    },
                    Instruction {
                        offset: 2,
                        opcode: 0x03,
                        payload: vec![],
                    },
                ],
            },
        );
        scripts.insert(
            "CHILD".into(),
            Script {
                code_start: 0,
                jumps: vec![],
                instructions: vec![
                    Instruction {
                        offset: 0,
                        opcode: 0x2B,
                        payload: vec![0, 3, 0xFF, b'A', 0],
                    },
                    Instruction {
                        offset: 1,
                        opcode: 0x85,
                        payload: vec![1, 0, 2, 0, 1, 4],
                    },
                    Instruction {
                        offset: 2,
                        opcode: 0x03,
                        payload: vec![],
                    },
                ],
            },
        );
        (
            Vm::new(scripts, "TEST").unwrap(),
            Graphics::new(PathBuf::from("PNG-images/GGD"), 2, 2),
        )
    }

    #[test]
    fn quicksave_round_trip_restores_state_and_pixels_and_rejects_wrong_scripts() {
        let path =
            std::env::temp_dir().join(format!("ikura-save-test-{}.json", std::process::id()));
        let (mut vm, mut graphics) = game();
        assert!(write(&path, 42, &vm, &graphics, AudioState::default()).is_err());
        vm.step().unwrap();
        vm.step().unwrap();
        vm.step().unwrap();
        graphics.fill_screen([12, 34, 56]);
        graphics.allocate(1, 3, 3).unwrap();
        graphics.fill_surface(1, [78, 90, 12]).unwrap();
        let expected = serde_json::to_value(&vm).unwrap();
        write(&path, 42, &vm, &graphics, AudioState::default()).unwrap();
        vm.click();
        vm.step().unwrap();
        graphics.fill_screen([255, 0, 0]);
        let changed = serde_json::to_value(&vm).unwrap();
        assert!(read(&path, 99, &mut vm, &mut graphics).is_err());
        assert_eq!(serde_json::to_value(&vm).unwrap(), changed);
        assert_eq!(graphics.screen().get_pixel(0, 0)[0], 255);
        read(&path, 42, &mut vm, &mut graphics).unwrap();
        assert_eq!(serde_json::to_value(&vm).unwrap(), expected);
        assert_eq!(graphics.screen().get_pixel(0, 0).0, [12, 34, 56, 255]);
        assert_eq!(vm.position(), Some(("CHILD", 1)));
        assert_eq!(
            graphics.surface(1).unwrap().get_pixel(0, 0).0,
            [78, 90, 12, 255]
        );
        write(&path, 42, &vm, &graphics, AudioState::default()).unwrap();
        let mut invalid: serde_json::Value =
            serde_json::from_reader(fs::File::open(&path).unwrap()).unwrap();
        invalid["vm"]["frames"][0]["pc"] = 999.into();
        fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
        assert!(read(&path, 42, &mut vm, &mut graphics).is_err());
        assert_eq!(serde_json::to_value(&vm).unwrap(), expected);
        fs::write(&path, "broken save").unwrap();
        assert!(read(&path, 42, &mut vm, &mut graphics).is_err());
        assert_eq!(serde_json::to_value(&vm).unwrap(), expected);
        fs::remove_file(path).unwrap();
    }
}
