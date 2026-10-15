//! Shared access to the original SM2MPX DATA cabinet.
use std::path::{Path, PathBuf};

fn dword(data: &[u8], offset: usize) -> Result<u32, String> {
    Ok(u32::from_le_bytes(
        data.get(offset..offset + 4)
            .ok_or("truncated DATA header")?
            .try_into()
            .unwrap(),
    ))
}

pub(crate) fn cabinet_entry<'a>(data: &'a [u8], name: &str) -> Result<&'a [u8], String> {
    if data.get(..8) != Some(b"SM2MPX10") {
        return Err("DATA is not an SM2MPX cabinet".into());
    }
    let count = dword(data, 8)? as usize;
    if count > 65536 || 32 + count * 20 > data.len() {
        return Err("invalid DATA directory".into());
    }
    for i in 0..count {
        let entry = 32 + i * 20;
        let filename = &data[entry..entry + 12];
        let end = filename.iter().position(|&b| b == 0).unwrap_or(12);
        if filename[..end].eq_ignore_ascii_case(name.as_bytes()) {
            let start = dword(data, entry + 12)? as usize;
            let size = dword(data, entry + 16)? as usize;
            return data
                .get(start..start.checked_add(size).ok_or("DATA entry overflow")?)
                .ok_or("truncated DATA entry".into());
        }
    }
    Err(format!("resource {name} missing from DATA"))
}

pub struct DataArchive {
    bytes: Vec<u8>,
}

impl DataArchive {
    pub fn load(data_dir: &Path, override_key: Option<&str>) -> Result<Option<Self>, String> {
        let override_path = override_key
            .and_then(std::env::var_os)
            .or_else(|| std::env::var_os("IKURA_DATA"));
        let candidates = if let Some(path) = override_path {
            vec![PathBuf::from(path)]
        } else {
            vec![
                data_dir.join("DATA"),
                data_dir.parent().unwrap_or(data_dir).join("DATA"),
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .parent()
                    .unwrap()
                    .join("DATA"),
            ]
        };
        let Some(path) = candidates.into_iter().find(|p| p.is_file()) else {
            return Ok(None);
        };
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if bytes.get(..8) != Some(b"SM2MPX10") {
            return Err(format!("{} is not an SM2MPX DATA cabinet", path.display()));
        }
        println!("DATA archive: {}", path.display());
        Ok(Some(Self { bytes }))
    }
    pub fn entry(&self, name: &str) -> Result<&[u8], String> {
        cabinet_entry(&self.bytes, name)
    }
}
