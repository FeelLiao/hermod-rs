//! 已验证文件保存在应用私有收件箱；前端只能传递不透明 ID。
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Serialize, Deserialize)]
pub struct Received {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

pub fn new_id() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("OS randomness unavailable");
    hermod_core::transfer::hex(&bytes)
}

fn directory(root: &Path, id: &str) -> Result<PathBuf, String> {
    if id.len() != 32 || !id.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("Invalid file ID".into());
    }
    Ok(root.join(id))
}

pub async fn record(
    directory: &Path,
    receipt: &hermod_core::orchestrate::Receipt,
) -> Result<(), String> {
    let parent = directory.parent().ok_or("Invalid inbox directory")?;
    let item = Received {
        id: parent
            .file_name()
            .ok_or("Invalid inbox ID")?
            .to_string_lossy()
            .into_owned(),
        name: receipt
            .path
            .file_name()
            .ok_or("Invalid filename")?
            .to_string_lossy()
            .into_owned(),
        size: receipt.size,
        sha256: receipt.sha256.clone(),
    };
    let encoded = serde_json::to_vec(&item).map_err(|e| e.to_string())?;
    let temporary = parent.join("receipt.tmp");
    tokio::fs::write(&temporary, encoded)
        .await
        .map_err(|e| e.to_string())?;
    tokio::fs::rename(temporary, parent.join("receipt.json"))
        .await
        .map_err(|e| e.to_string())
}

pub fn resolve(root: &Path, id: &str) -> Result<(Received, PathBuf), String> {
    let directory = directory(root, id)?;
    let item: Received = serde_json::from_slice(
        &std::fs::read(directory.join("receipt.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    if item.id != id {
        return Err("Invalid receipt ID".into());
    }
    if item.name.is_empty()
        || item.name == "."
        || item.name == ".."
        || item.name.contains(['/', '\\', '\0'])
    {
        return Err("Invalid receipt filename".into());
    }
    let path = directory.join("payload").join(&item.name);
    let canonical = path.canonicalize().map_err(|e| e.to_string())?;
    let base = directory
        .join("payload")
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if canonical.parent() != Some(base.as_path()) || !canonical.is_file() {
        return Err("Invalid inbox file".into());
    }
    Ok((item, canonical))
}

pub fn list(root: &Path) -> Result<Vec<Received>, String> {
    let mut result = Vec::new();
    for entry in std::fs::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if let Ok((item, _)) = resolve(root, &entry.file_name().to_string_lossy()) {
            result.push(item);
        }
    }
    result.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(result)
}

/// 仅在进程启动且获得单实例锁后回收没有校验收据的中断接收。
pub fn clean_incomplete(root: &Path) -> Result<(), String> {
    for entry in std::fs::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let id = entry.file_name();
        if directory(root, &id.to_string_lossy()).is_ok()
            && entry.file_type().map_err(|e| e.to_string())?.is_dir()
            && !entry.path().join("receipt.json").exists()
        {
            std::fs::remove_dir_all(entry.path()).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

pub fn remove(root: &Path, id: &str) -> Result<(), String> {
    resolve(root, id)?;
    std::fs::remove_dir_all(directory(root, id)?).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn only_verified_receipts_are_listed_and_names_cannot_escape() {
        let root = std::env::temp_dir().join(format!("hermod-inbox-{}", new_id()));
        let id = new_id();
        let payload = root.join(&id).join("payload");
        std::fs::create_dir_all(&payload).unwrap();
        let path = payload.join("地球.zip");
        std::fs::write(&path, b"hello").unwrap();
        assert!(list(&root).unwrap().is_empty());
        record(
            &payload,
            &hermod_core::orchestrate::Receipt {
                path,
                size: 5,
                sha256: hermod_core::transfer::hash_bytes(b"hello"),
                kind: hermod_core::transfer::Kind::File,
            },
        )
        .await
        .unwrap();
        let interrupted = root.join(new_id());
        std::fs::create_dir_all(interrupted.join("payload")).unwrap();
        std::fs::write(interrupted.join("payload/partial.hermod_tmp"), b"partial").unwrap();
        clean_incomplete(&root).unwrap();
        assert!(!interrupted.exists());
        assert_eq!(list(&root).unwrap()[0].name, "地球.zip");
        assert!(resolve(&root, "../outside").is_err());
        let mut item = list(&root).unwrap().remove(0);
        item.name = "../../outside".into();
        std::fs::write(
            root.join(&id).join("receipt.json"),
            serde_json::to_vec(&item).unwrap(),
        )
        .unwrap();
        assert!(resolve(&root, &id).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
