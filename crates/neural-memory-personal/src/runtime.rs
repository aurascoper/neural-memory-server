use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use ed25519_dalek::{SigningKey, VerifyingKey};

use crate::{signer_key_id, PersonalStore, SyncCursor};

#[derive(Debug, PartialEq, Eq)]
pub struct RotatedKey {
    pub old_key_id: String,
    pub new_key_id: String,
}

pub fn parse_cursor(value: &str) -> Result<SyncCursor, String> {
    let (epoch, sequence) = value.split_once(':').ok_or("cursor must be E:S")?;
    if epoch.is_empty()
        || sequence.is_empty()
        || !epoch.bytes().all(|byte| byte.is_ascii_digit())
        || !sequence.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err("cursor must be decimal E:S".into());
    }
    Ok(SyncCursor {
        epoch: epoch.parse().map_err(|_| "epoch out of range")?,
        sequence: sequence.parse().map_err(|_| "sequence out of range")?,
    })
}

pub fn store_now(store: &PersonalStore) -> Result<String, String> {
    store
        .conn
        .query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ','now')", [], |row| {
            row.get(0)
        })
        .map_err(|error| error.to_string())
}

pub fn load_or_create_signing_key(path: &Path) -> Result<SigningKey, String> {
    let mut bytes = [0_u8; 32];
    match open_secret(path) {
        Ok(mut file) => {
            require_secure_file(&file, path, effective_uid())?;
            file.read_exact(&mut bytes)
                .map_err(|error| error.to_string())?;
            let mut extra = [0_u8; 1];
            if file.read(&mut extra).map_err(|error| error.to_string())? != 0 {
                return Err("signing key must be exactly 32 bytes".into());
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            File::open("/dev/urandom")
                .and_then(|mut file| file.read_exact(&mut bytes))
                .map_err(|error| format!("randomness: {error}"))?;
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)
                .map_err(|error| format!("create {}: {error}", path.display()))?;
            file.write_all(&bytes).map_err(|error| error.to_string())?;
            file.sync_all().map_err(|error| error.to_string())?;
            require_secure_file(&file, path, effective_uid())?;
        }
        Err(error) => return Err(format!("open {}: {error}", path.display())),
    }
    Ok(SigningKey::from_bytes(&bytes))
}

pub fn load_existing_signing_key(path: &Path) -> Result<SigningKey, String> {
    let mut bytes = [0_u8; 32];
    let mut file =
        open_secret(path).map_err(|error| format!("open {}: {error}", path.display()))?;
    require_secure_file(&file, path, effective_uid())?;
    file.read_exact(&mut bytes)
        .map_err(|error| error.to_string())?;
    let mut extra = [0_u8; 1];
    if file.read(&mut extra).map_err(|error| error.to_string())? != 0 {
        return Err("signing key must be exactly 32 bytes".into());
    }
    Ok(SigningKey::from_bytes(&bytes))
}

pub fn rotate_signing_key(path: &Path) -> Result<RotatedKey, String> {
    let parent = path
        .parent()
        .ok_or("signing key must have a parent directory")?;
    let parent_metadata = fs::symlink_metadata(parent).map_err(|error| error.to_string())?;
    if parent_metadata.file_type().is_symlink()
        || !parent_metadata.is_dir()
        || parent_metadata.uid() != effective_uid()
        || parent_metadata.permissions().mode() & 0o777 != 0o700
    {
        return Err("signing key parent must be a mode-0700 owned directory, not a symlink".into());
    }
    let old = load_existing_signing_key(path)?;
    let mut bytes = [0_u8; 32];
    File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(|error| format!("randomness: {error}"))?;
    let new = SigningKey::from_bytes(&bytes);
    let name = path.file_name().ok_or("signing key must name a file")?;
    let temporary = parent.join(format!(
        ".{}.rotate-{}",
        name.to_string_lossy(),
        std::process::id()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&temporary)
        .map_err(|error| format!("create rotation file: {error}"))?;
    file.write_all(&bytes).map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    require_secure_file(&file, &temporary, effective_uid())?;
    drop(file);
    fs::rename(&temporary, path).map_err(|error| format!("install rotated key: {error}"))?;
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("sync key directory: {error}"))?;
    Ok(RotatedKey {
        old_key_id: signer_key_id(&old.verifying_key()),
        new_key_id: signer_key_id(&new.verifying_key()),
    })
}

pub fn decode_verifying_key(value: &str) -> Result<VerifyingKey, String> {
    let bytes = BASE64
        .decode(value)
        .map_err(|error| format!("trusted key base64: {error}"))?;
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| "trusted key must decode to 32 bytes")?;
    VerifyingKey::from_bytes(&bytes).map_err(|error| format!("trusted key: {error}"))
}

pub fn load_enrolled_verifying_key(path: &Path) -> Result<VerifyingKey, String> {
    let mut bytes = [0_u8; 32];
    let mut file =
        open_secret(path).map_err(|error| format!("open {}: {error}", path.display()))?;
    require_secure_file(&file, path, effective_uid())?;
    file.read_exact(&mut bytes)
        .map_err(|error| error.to_string())?;
    let mut extra = [0_u8; 1];
    if file.read(&mut extra).map_err(|error| error.to_string())? != 0 {
        return Err("enrolled public key must be exactly 32 bytes".into());
    }
    VerifyingKey::from_bytes(&bytes).map_err(|error| format!("enrolled public key: {error}"))
}

pub fn enroll_peer_verifying_key(path: &Path, key: &VerifyingKey) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or("peer key must have a parent directory")?;
    let metadata = fs::symlink_metadata(parent).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || metadata.uid() != effective_uid()
        || metadata.permissions().mode() & 0o777 != 0o700
    {
        return Err("peer key parent must be a mode-0700 owned directory, not a symlink".into());
    }
    if path.exists() {
        load_enrolled_verifying_key(path)?;
    }
    let name = path.file_name().ok_or("peer key must name a file")?;
    let temporary = parent.join(format!(
        ".{}.enroll-{}",
        name.to_string_lossy(),
        std::process::id()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&temporary)
        .map_err(|error| format!("create enrollment file: {error}"))?;
    file.write_all(key.as_bytes())
        .map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    require_secure_file(&file, &temporary, effective_uid())?;
    drop(file);
    fs::rename(&temporary, path).map_err(|error| format!("install enrolled key: {error}"))?;
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("sync key directory: {error}"))
}

pub fn load_bearer_token(path: &Path) -> Result<Vec<u8>, String> {
    let file = open_secret(path).map_err(|error| format!("open {}: {error}", path.display()))?;
    require_secure_file(&file, path, effective_uid())?;
    let mut token = Vec::new();
    file.take(4097)
        .read_to_end(&mut token)
        .map_err(|error| error.to_string())?;
    while token
        .last()
        .is_some_and(|byte| matches!(byte, b'\n' | b'\r'))
    {
        token.pop();
    }
    let decoded = BASE64
        .decode(&token)
        .map_err(|_| "bearer token must be canonical base64 for 32 random bytes".to_string())?;
    if decoded.len() != 32 || BASE64.encode(decoded).as_bytes() != token {
        return Err("bearer token must be canonical base64 for 32 random bytes".into());
    }
    Ok(token)
}

fn open_secret(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
}

fn effective_uid() -> u32 {
    // SAFETY: geteuid has no preconditions and does not dereference pointers.
    unsafe { libc::geteuid() }
}

fn require_secure_file(file: &File, path: &Path, expected_uid: u32) -> Result<(), String> {
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.file_type().is_file() {
        return Err(format!("{} must be a regular file", path.display()));
    }
    let mode = metadata.permissions().mode() & 0o777;
    if mode != 0o600 {
        return Err(format!("{} must have mode 0600", path.display()));
    }
    if metadata.uid() != expected_uid {
        return Err(format!(
            "{} must be owned by effective user {}",
            path.display(),
            expected_uid
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::ffi::CString;
    use std::fs;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{symlink, PermissionsExt};

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn secret_loaders_reject_symlinks_fifos_modes_and_wrong_owners() {
        let directory = tempdir().unwrap();
        let key = directory.path().join("key");
        fs::write(&key, [7_u8; 32]).unwrap();
        fs::set_permissions(&key, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(load_or_create_signing_key(&key).is_ok());

        let link = directory.path().join("link");
        symlink(&key, &link).unwrap();
        assert!(load_or_create_signing_key(&link).is_err());
        assert!(load_bearer_token(&link).is_err());

        let fifo = directory.path().join("fifo");
        let fifo_path = CString::new(fifo.as_os_str().as_bytes()).unwrap();
        // SAFETY: fifo_path is a valid, NUL-terminated path and mode is valid.
        assert_eq!(unsafe { libc::mkfifo(fifo_path.as_ptr(), 0o600) }, 0);
        assert!(load_or_create_signing_key(&fifo).is_err());
        assert!(load_bearer_token(&fifo).is_err());

        fs::set_permissions(&key, fs::Permissions::from_mode(0o640)).unwrap();
        assert!(load_or_create_signing_key(&key).is_err());
        fs::set_permissions(&key, fs::Permissions::from_mode(0o600)).unwrap();
        let file = open_secret(&key).unwrap();
        assert!(require_secure_file(&file, &key, effective_uid().wrapping_add(1)).is_err());
    }

    #[test]
    fn signing_key_rotation_is_atomic_private_and_rejects_unsafe_parent() {
        let directory = tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let key = directory.path().join("key");
        fs::write(&key, [3_u8; 32]).unwrap();
        fs::set_permissions(&key, fs::Permissions::from_mode(0o600)).unwrap();
        let old = signer_key_id(&load_existing_signing_key(&key).unwrap().verifying_key());
        let rotated = rotate_signing_key(&key).unwrap();
        assert_eq!(rotated.old_key_id, old);
        assert_ne!(rotated.old_key_id, rotated.new_key_id);
        assert_eq!(
            rotated.new_key_id,
            signer_key_id(&load_existing_signing_key(&key).unwrap().verifying_key())
        );
        assert_eq!(
            fs::metadata(&key).unwrap().permissions().mode() & 0o777,
            0o600
        );

        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o755)).unwrap();
        let before = fs::read(&key).unwrap();
        assert!(rotate_signing_key(&key).is_err());
        assert_eq!(fs::read(&key).unwrap(), before);

        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let link = directory.path().join("key-link");
        symlink(&key, &link).unwrap();
        assert!(rotate_signing_key(&link).is_err());
    }

    #[test]
    fn enrolled_peer_key_is_private_atomic_and_not_a_symlink() {
        let directory = tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let path = directory.path().join("mac.pub");
        let first = SigningKey::from_bytes(&[4; 32]).verifying_key();
        enroll_peer_verifying_key(&path, &first).unwrap();
        assert_eq!(load_enrolled_verifying_key(&path).unwrap(), first);
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        assert!(load_enrolled_verifying_key(&path).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let link = directory.path().join("mac-link.pub");
        symlink(&path, &link).unwrap();
        assert!(load_enrolled_verifying_key(&link).is_err());
        assert!(enroll_peer_verifying_key(&link, &first).is_err());
    }
}
