use crate::{
    graphics::{Graphics, GraphicsSnapshot},
    vm::Vm,
};
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use std::{
    error::Error,
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

const FORMAT_VERSION: u32 = 1;
const MAGIC: &[u8; 8] = b"IKRSAVE2";
pub const SLOT_COUNT: u8 = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Quick,
    Auto,
    Manual(u8),
}

pub fn slot_path(root: &Path, slot: Slot) -> PathBuf {
    match slot {
        Slot::Quick => root.join("quicksave.iks"),
        Slot::Auto => root.join("autosave.iks"),
        Slot::Manual(number) => {
            assert!((1..=SLOT_COUNT).contains(&number));
            root.join(format!("slot-{number:02}.iks"))
        }
    }
}

pub fn load_path(root: &Path, slot: Slot) -> PathBuf {
    let path = slot_path(root, slot);
    let legacy = root.join("quicksave.json");
    if slot == Slot::Quick && !path.exists() && !backup_path(&path).exists() && legacy.exists() {
        legacy
    } else {
        path
    }
}

pub fn delete_slot(root: &Path, slot: Slot) -> Result<(), Box<dyn Error>> {
    let path = slot_path(root, slot);
    let mut paths = vec![backup_path(&path), path];
    if slot == Slot::Quick {
        paths.push(root.join("quicksave.json"));
    }
    for path in paths {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

pub fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".bak");
    PathBuf::from(name)
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Metadata {
    pub script_signature: u64,
    pub saved_at: u64,
    pub chapter: String,
    pub excerpt: String,
    pub script: String,
    pub offset: usize,
    pub thumbnail: Vec<u8>,
}

#[derive(Default, Serialize, Deserialize)]
pub struct AudioState {
    pub music: Option<PathBuf>,
    pub voice: Option<PathBuf>,
    #[serde(default)]
    pub music_position_ms: u64,
    #[serde(default)]
    pub voice_position_ms: u64,
}

#[derive(Serialize)]
struct SaveRef<'a> {
    version: u32,
    script_signature: u64,
    vm: &'a Vm,
    graphics: GraphicsSnapshot,
    audio: AudioState,
    session: &'a crate::history::Session,
}

#[derive(Serialize, Deserialize)]
struct SaveFile {
    version: u32,
    script_signature: u64,
    vm: Vm,
    graphics: GraphicsSnapshot,
    audio: AudioState,
    #[serde(default)]
    session: crate::history::Session,
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
    write_with_preview(path, signature, vm, graphics, audio, graphics.screen())
}

pub fn write_with_preview(
    path: &Path,
    signature: u64,
    vm: &Vm,
    graphics: &Graphics,
    audio: AudioState,
    preview: &RgbaImage,
) -> Result<(), Box<dyn Error>> {
    write_with_session(
        path,
        signature,
        vm,
        graphics,
        audio,
        preview,
        &crate::history::Session::default(),
    )
}

pub fn write_with_session(
    path: &Path,
    signature: u64,
    vm: &Vm,
    graphics: &Graphics,
    mut audio: AudioState,
    preview: &RgbaImage,
    session: &crate::history::Session,
) -> Result<(), Box<dyn Error>> {
    session.validate()?;
    if !vm.can_save() {
        return Err("the script currently disables saving".into());
    }
    let root = graphics.data_root();
    for path in [&mut audio.music, &mut audio.voice].into_iter().flatten() {
        if let Ok(relative) = path.strip_prefix(&root) {
            *path = relative.to_path_buf();
        }
    }
    let mut png = std::io::Cursor::new(Vec::new());
    image::imageops::thumbnail(preview, 112, 84).write_to(&mut png, image::ImageFormat::Png)?;
    let (script, offset) = vm.position().ok_or("no active script")?;
    let metadata = Metadata {
        script_signature: signature,
        saved_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs(),
        chapter: vm.save_title().unwrap_or(script).into(),
        excerpt: vm.text_page().map_or_else(
            || "Choice".into(),
            |p| {
                p.render()
                    .chars()
                    .take(100)
                    .collect::<String>()
                    .replace('\n', " ")
            },
        ),
        script: script.into(),
        offset,
        thumbnail: png.into_inner(),
    };
    let header = serde_json::to_vec(&metadata)?;
    let state = SaveRef {
        version: FORMAT_VERSION,
        script_signature: signature,
        vm,
        graphics: graphics.snapshot()?,
        audio,
        session,
    };
    let parent = path.parent().ok_or("save path has no parent")?;
    fs::create_dir_all(parent)?;
    let temporary = temporary_path(path);
    let result = (|| -> Result<(), Box<dyn Error>> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(MAGIC)?;
        file.write_all(&(header.len() as u32).to_le_bytes())?;
        file.write_all(&header)?;
        let mut compressed = GzEncoder::new(file, Compression::fast());
        serde_json::to_writer(&mut compressed, &state)?;
        let mut file = compressed.finish()?;
        file.flush()?;
        file.sync_all()?;
        // Keep the last syntactically valid save; do not replace a good backup
        // with a truncated/corrupt primary file.
        if path.exists() && decode_file(path).is_ok() {
            let backup = backup_path(path);
            let backup_temp = temporary_path(&backup);
            if let Err(error) = (|| -> Result<(), Box<dyn Error>> {
                fs::copy(path, &backup_temp)?;
                fs::File::open(&backup_temp)?.sync_all()?;
                fs::rename(&backup_temp, backup)?;
                Ok(())
            })() {
                let _ = fs::remove_file(&backup_temp);
                return Err(error);
            }
        }
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    PathBuf::from(name)
}

fn open_payload(path: &Path) -> Result<(Option<Metadata>, fs::File), Box<dyn Error>> {
    let mut file = fs::File::open(path)?;
    let mut magic = [0; 8];
    file.read_exact(&mut magic)?;
    if &magic != MAGIC {
        file.seek(SeekFrom::Start(0))?;
        return Ok((None, file));
    }
    let mut length = [0; 4];
    file.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length) as usize;
    if length > 1024 * 1024 {
        return Err("save metadata is too large".into());
    }
    let mut metadata = vec![0; length];
    file.read_exact(&mut metadata)?;
    Ok((Some(serde_json::from_slice(&metadata)?), file))
}

fn decode_file(path: &Path) -> Result<SaveFile, Box<dyn Error>> {
    let (metadata, file) = open_payload(path)?;
    let state = if metadata.is_some() {
        serde_json::from_reader(GzDecoder::new(file).take(512 * 1024 * 1024))?
    } else {
        serde_json::from_reader(file)?
    };
    Ok(state)
}

pub fn metadata(path: &Path) -> Result<Metadata, Box<dyn Error>> {
    let (metadata, _) = open_payload(path)?;
    if let Some(metadata) = metadata {
        return Ok(metadata);
    }
    // Legacy JSON saves have no quick header. Only old quicksaves take this
    // slower path; modern slots never decode VRAM just to list the menu.
    let state = decode_file(path)?;
    let (script, offset) = state.vm.position().ok_or("save has no active script")?;
    Ok(Metadata {
        script_signature: state.script_signature,
        saved_at: fs::metadata(path)?
            .modified()?
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs(),
        chapter: state.vm.save_title().unwrap_or(script).into(),
        excerpt: state
            .vm
            .text_page()
            .map_or_else(String::new, |p| p.render()),
        script: script.into(),
        offset,
        thumbnail: vec![],
    })
}

#[allow(dead_code)] // Compatibility helper for callers that do not need saved history.
pub fn read_with_backup(
    path: &Path,
    signature: u64,
    vm: &mut Vm,
    graphics: &mut Graphics,
) -> Result<(AudioState, bool), Box<dyn Error>> {
    read_session_with_backup(path, signature, vm, graphics)
        .map(|(audio, _, recovered)| (audio, recovered))
}
pub fn read_session_with_backup(
    path: &Path,
    signature: u64,
    vm: &mut Vm,
    graphics: &mut Graphics,
) -> Result<(AudioState, crate::history::Session, bool), Box<dyn Error>> {
    match read_full(path, signature, vm, graphics) {
        Ok((audio, session)) => Ok((audio, session, false)),
        Err(primary) => {
            let backup = backup_path(path);
            if !backup.exists() {
                return Err(primary);
            }
            read_full(&backup, signature, vm, graphics)
                .map(|(a, s)| (a, s, true))
                .map_err(|e| format!("save failed: {primary}; backup failed: {e}").into())
        }
    }
}

pub fn read(
    path: &Path,
    signature: u64,
    vm: &mut Vm,
    graphics: &mut Graphics,
) -> Result<AudioState, Box<dyn Error>> {
    read_full(path, signature, vm, graphics).map(|(audio, _)| audio)
}

fn read_full(
    path: &Path,
    signature: u64,
    vm: &mut Vm,
    graphics: &mut Graphics,
) -> Result<(AudioState, crate::history::Session), Box<dyn Error>> {
    let mut state = decode_file(path)?;
    state.session.validate()?;
    if state.version != FORMAT_VERSION {
        return Err("unsupported save format version".into());
    }
    if state.script_signature != signature {
        return Err("save belongs to a different script export".into());
    }
    let root = graphics.data_root();
    for path in [&mut state.audio.music, &mut state.audio.voice]
        .into_iter()
        .flatten()
    {
        if path.is_relative() {
            *path = root.join(&*path);
        }
        if !path.is_file()
            && let Some(index) = path
                .components()
                .position(|part| part.as_os_str() == "OGG-audio")
        {
            let relative: PathBuf = path.components().skip(index).collect();
            *path = root.join(relative);
        }
        if !path.is_file() {
            return Err(format!("saved audio is missing: {}", path.display()).into());
        }
    }
    // Decode and validate all surfaces before changing either live component.
    let restored_graphics = graphics.restored(state.graphics)?;
    vm.restore(state.vm)?;
    *graphics = restored_graphics;
    Ok((state.audio, state.session))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::{Instruction, Script, Scripts};

    #[test]
    fn automatic_slot_restores_history_positions_and_delete_removes_backups() {
        let root = std::env::temp_dir().join(format!("ikura-session-test-{}", std::process::id()));
        let (mut vm, mut graphics) = game();
        for _ in 0..3 {
            vm.step().unwrap();
        }
        let session = crate::history::Session {
            history: vec![crate::history::Entry {
                page: crate::vm::TextPage {
                    speaker: Some("Test".into()),
                    body: "Saved history".into(),
                },
                voices: vec![PathBuf::from("OGG-audio/VOICE1/V001.OGG")],
            }],
        };
        let path = slot_path(&root, Slot::Auto);
        for _ in 0..2 {
            write_with_session(
                &path,
                42,
                &vm,
                &graphics,
                AudioState {
                    music_position_ms: 1234,
                    voice_position_ms: 567,
                    ..AudioState::default()
                },
                graphics.screen(),
                &session,
            )
            .unwrap();
        }
        assert!(backup_path(&path).exists());
        let (audio, restored, recovered) =
            read_session_with_backup(&path, 42, &mut vm, &mut graphics).unwrap();
        assert!(!recovered);
        assert_eq!(audio.music_position_ms, 1234);
        assert_eq!(audio.voice_position_ms, 567);
        assert_eq!(restored.history[0].page.body, "Saved history");
        assert_eq!(restored.history[0].voices, session.history[0].voices);
        delete_slot(&root, Slot::Auto).unwrap();
        assert!(!path.exists());
        assert!(!backup_path(&path).exists());
        assert!(read_with_backup(&path, 42, &mut vm, &mut graphics).is_err());
        let _ = fs::remove_dir_all(root);
    }

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
    fn save_at_a_choice_restores_the_pending_options_and_can_choose_again() {
        let root =
            std::env::temp_dir().join(format!("ikura-choice-save-test-{}", std::process::id()));
        let path = slot_path(&root, Slot::Manual(3));
        let option = |value: u8, caption: u8| {
            let mut data = vec![0, value];
            for coordinate in [10u32, 20, 100, 30] {
                data.extend(coordinate.to_le_bytes());
            }
            data.extend([caption, 0]);
            data
        };
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
                        opcode: 0x15,
                        payload: option(0, b'A'),
                    },
                    Instruction {
                        offset: 2,
                        opcode: 0x15,
                        payload: option(1, b'B'),
                    },
                    Instruction {
                        offset: 3,
                        opcode: 0x18,
                        payload: vec![0, 7, 0, 0],
                    },
                    Instruction {
                        offset: 4,
                        opcode: 0x03,
                        payload: vec![],
                    },
                ],
            },
        );
        let mut vm = Vm::new(scripts, "TEST").unwrap();
        let mut graphics = Graphics::new(root.join("PNG-images/GGD"), 2, 2);
        for _ in 0..4 {
            vm.step().unwrap();
        }
        assert!(vm.waiting_for_choice());
        write(&path, 42, &vm, &graphics, AudioState::default()).unwrap();
        vm.choose(1).unwrap();
        vm.step().unwrap();
        assert!(!vm.waiting_for_choice());
        read(&path, 42, &mut vm, &mut graphics).unwrap();
        assert!(vm.waiting_for_choice());
        assert_eq!(vm.choice_options().len(), 2);
        assert!(vm.can_save());
        vm.choose(0).unwrap();
        vm.step().unwrap();
        assert_eq!(serde_json::to_value(&vm).unwrap()["variables"]["7"], 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn compressed_saves_have_metadata_and_recover_a_valid_backup() {
        let root = std::env::temp_dir().join(format!("ikura-backup-test-{}", std::process::id()));
        let path = slot_path(&root, Slot::Manual(1));
        let (mut vm, mut graphics) = game();
        for _ in 0..3 {
            vm.step().unwrap();
        }
        graphics.fill_screen([10, 20, 30]);
        write(&path, 42, &vm, &graphics, AudioState::default()).unwrap();
        assert_eq!(&fs::read(&path).unwrap()[..8], MAGIC);
        let info = metadata(&path).unwrap();
        assert_eq!(info.excerpt, "A");
        assert_eq!(info.script, "CHILD");
        assert_eq!(info.script_signature, 42);
        assert!(info.saved_at > 0);
        assert!(image::load_from_memory(&info.thumbnail).is_ok());
        graphics.fill_screen([80, 90, 100]);
        write(&path, 42, &vm, &graphics, AudioState::default()).unwrap();
        assert!(backup_path(&path).exists());
        let mut broken = fs::read(&path).unwrap();
        broken.truncate(broken.len() - 10);
        fs::write(&path, broken).unwrap();
        assert!(read(&path, 42, &mut vm, &mut graphics).is_err());
        let (_, recovered) = read_with_backup(&path, 42, &mut vm, &mut graphics).unwrap();
        assert!(recovered);
        assert_eq!(graphics.screen().get_pixel(0, 0).0, [10, 20, 30, 255]);
        // Replacing a corrupt primary must preserve its still-valid backup.
        write(&path, 42, &vm, &graphics, AudioState::default()).unwrap();
        assert!(read(&backup_path(&path), 42, &mut vm, &mut graphics).is_ok());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_json_quicksaves_and_moved_audio_paths_remain_readable() {
        let root = std::env::temp_dir().join(format!("ikura-legacy-test-{}", std::process::id()));
        fs::create_dir_all(root.join("OGG-audio/WMSC")).unwrap();
        let music = root.join("OGG-audio/WMSC/25.OGG");
        fs::write(&music, b"path resolution fixture").unwrap();
        let (mut vm, _) = game();
        let mut graphics = Graphics::new(root.join("PNG-images/GGD"), 2, 2);
        for _ in 0..3 {
            vm.step().unwrap();
        }
        let path = root.join("quicksave.json");
        let legacy = SaveRef {
            session: &crate::history::Session::default(),
            version: 1,
            script_signature: 42,
            vm: &vm,
            graphics: graphics.snapshot().unwrap(),
            audio: AudioState {
                music: Some(PathBuf::from("/old-game/OGG-audio/WMSC/25.OGG")),
                voice: None,
                ..AudioState::default()
            },
        };
        let mut old_json = serde_json::to_value(&legacy).unwrap();
        old_json.as_object_mut().unwrap().remove("session");
        old_json["audio"]
            .as_object_mut()
            .unwrap()
            .remove("music_position_ms");
        old_json["audio"]
            .as_object_mut()
            .unwrap()
            .remove("voice_position_ms");
        fs::write(&path, serde_json::to_vec(&old_json).unwrap()).unwrap();
        assert_eq!(load_path(&root, Slot::Quick), path);
        let audio = read(&path, 42, &mut vm, &mut graphics).unwrap();
        assert_eq!(audio.music, Some(music.clone()));
        let modern = slot_path(&root, Slot::Quick);
        write(
            &modern,
            42,
            &vm,
            &graphics,
            AudioState {
                music: Some(music.clone()),
                voice: None,
                ..AudioState::default()
            },
        )
        .unwrap();
        assert_eq!(
            decode_file(&modern).unwrap().audio.music,
            Some(PathBuf::from("OGG-audio/WMSC/25.OGG"))
        );
        assert_eq!(load_path(&root, Slot::Quick), modern);
        assert_eq!(
            read(&modern, 42, &mut vm, &mut graphics).unwrap().music,
            Some(music)
        );
        assert!(path.exists(), "legacy save is retained");
        fs::remove_dir_all(root).unwrap();
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
            serde_json::to_value(decode_file(&path).unwrap()).unwrap();
        invalid["vm"]["frames"][0]["pc"] = 999.into();
        fs::write(&path, serde_json::to_vec(&invalid).unwrap()).unwrap();
        assert!(read(&path, 42, &mut vm, &mut graphics).is_err());
        assert_eq!(serde_json::to_value(&vm).unwrap(), expected);
        fs::write(&path, "broken save").unwrap();
        assert!(read(&path, 42, &mut vm, &mut graphics).is_err());
        assert_eq!(serde_json::to_value(&vm).unwrap(), expected);
        let _ = fs::remove_file(backup_path(&path));
        fs::remove_file(path).unwrap();
    }
}
