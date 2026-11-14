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
    record_verified(directory, receipt)
}

/// 同步提交是取消线性化边界：接收返回校验凭据后，不再在持久化期间 yield。
/// 元数据写入失败会留下隔离标记/最终载荷；启动清理必须保留，且没有有效
/// receipt.json 时不得在收件箱展示。当前不提供自动恢复或自动删除隔离载荷。
pub fn record_verified(
    directory: &Path,
    receipt: &hermod_core::orchestrate::Receipt,
) -> Result<(), String> {
    use std::io::Write;
    let parent = directory.parent().ok_or("Invalid inbox directory")?;
    // 提交失败时也不得将已校验载荷当成中断下载清理。
    let marker =
        std::fs::File::create(parent.join("verified.pending")).map_err(|e| e.to_string())?;
    marker.sync_all().map_err(|e| e.to_string())?;
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
    let mut file = std::fs::File::create(&temporary).map_err(|e| e.to_string())?;
    file.write_all(&encoded).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    std::fs::rename(temporary, parent.join("receipt.json")).map_err(|e| e.to_string())?;
    // 留存 marker 无害；仅在收据已提交后尝试清理它。
    let _ = std::fs::remove_file(parent.join("verified.pending"));
    Ok(())
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
fn has_finalized_payload(payload: &Path) -> Result<bool, String> {
    let entries = match std::fs::read_dir(payload) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.to_string()),
    };
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_name().to_string_lossy().ends_with(".hermod_tmp") {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn clean_incomplete(root: &Path) -> Result<(), String> {
    for entry in std::fs::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let id = entry.file_name();
        if directory(root, &id.to_string_lossy()).is_ok()
            && entry.file_type().map_err(|e| e.to_string())?.is_dir()
            && !entry.path().join("receipt.json").exists()
            && !entry.path().join("verified.pending").exists()
            // A finalized payload without metadata may be a verified commit failure.
            // Only discard directories containing exclusively core temporary files.
            && !has_finalized_payload(&entry.path().join("payload"))?
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
    #[test]
    fn verified_commit_survives_cancelled_cleanup_and_failed_metadata_commit() {
        let root = std::env::temp_dir().join(format!("hermod-verified-{}", new_id()));
        let id = new_id();
        let payload = root.join(&id).join("payload");
        std::fs::create_dir_all(&payload).unwrap();
        let path = payload.join("verified.bin");
        std::fs::write(&path, b"verified").unwrap();
        let receipt = hermod_core::orchestrate::Receipt {
            path: path.clone(),
            size: 8,
            sha256: hermod_core::transfer::hash_bytes(b"verified"),
            kind: hermod_core::transfer::Kind::File,
        };
        // Even failure to create the quarantine marker must not delete finalized data.
        clean_incomplete(&root).unwrap();
        assert!(path.exists());
        // A metadata write failure must quarantine, not delete, verified data.
        std::fs::create_dir(root.join(&id).join("receipt.tmp")).unwrap();
        assert!(record_verified(&payload, &receipt).is_err());
        clean_incomplete(&root).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"verified");
        std::fs::remove_dir(root.join(&id).join("receipt.tmp")).unwrap();
        record_verified(&payload, &receipt).unwrap();
        // Model cancellation or failed connection cleanup after synchronous commit.
        clean_incomplete(&root).unwrap();
        assert_eq!(resolve(&root, &id).unwrap().0.size, 8);
        assert_eq!(std::fs::read(&path).unwrap(), b"verified");
        std::fs::remove_dir_all(root).unwrap();
    }

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
