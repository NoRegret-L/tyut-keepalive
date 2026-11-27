//! Platform secret storage. Only the scheduler thread accesses the store.
//! The single secure record contains the protocol-encoded account and password.
//! XOR is a wire-format compatibility mechanism, never the storage protection.
use anyhow::{anyhow, bail, Result};
use std::{fs, path::Path};
use super::config::{self, Credentials};

const SERVICE: &str = crate::APP_IDENTIFIER;
const RECORD: &str = "campus-credentials";

trait Store {
    fn read(&self) -> Result<Option<String>>;
    fn write(&self, value: &str) -> Result<()>;
    fn delete(&self) -> Result<()>;
}

struct NativeStore(keyring::Entry);

impl NativeStore {
    fn open() -> Result<Self> {
        // Explicit native backend features are enabled in Cargo.toml.
        // Do not silently use keyring's mock store on unsupported platforms.
        #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
        bail!("此平台没有配置系统凭据库");
        #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
        {
            Ok(Self(keyring::Entry::new(SERVICE, RECORD)
                .map_err(|_| anyhow!("无法打开系统凭据库"))?))
        }
    }
}

impl Store for NativeStore {
    fn read(&self) -> Result<Option<String>> {
        match self.0.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => bail!("无法读取系统凭据库，请检查是否已解锁"),
        }
    }
    fn write(&self, value: &str) -> Result<()> {
        self.0.set_password(value).map_err(|_| anyhow!("系统凭据库保存失败"))
    }
    fn delete(&self) -> Result<()> {
        match self.0.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => bail!("系统凭据库删除失败，凭据可能仍然存在"),
        }
    }
}

fn decode(value: &str) -> Result<Credentials> {
    let value: Credentials = serde_json::from_str(value)
        .map_err(|_| anyhow!("系统凭据记录格式无效"))?;
    config::validate(&value)?;
    Ok(value)
}

fn remove_legacy(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => bail!("旧凭据文件删除失败，请检查文件权限后重试"),
    }
}

fn write_verified(store: &impl Store, value: &Credentials) -> Result<()> {
    config::validate(value)?;
    let serialized = serde_json::to_string(value)?;
    store.write(&serialized)?;
    let stored = store.read()?.ok_or_else(|| anyhow!("系统凭据保存后无法读取"))?;
    if stored != serialized {
        bail!("系统凭据写入校验失败");
    }
    Ok(())
}

fn clear_marker(legacy: &Path) -> std::path::PathBuf {
    legacy.with_file_name("credentials-clear-pending")
}
fn marker_present(legacy: &Path) -> Result<bool> {
    match fs::symlink_metadata(clear_marker(legacy)) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => bail!("无法检查凭据清除状态，已停止自动认证"),
    }
}
fn mark_clear(legacy: &Path) -> Result<()> {
    // The marker contains no secret. create_new also avoids following symlinks.
    if marker_present(legacy)? { return Ok(()); }
    let file = fs::OpenOptions::new().write(true).create_new(true).open(clear_marker(legacy))
        .map_err(|_| anyhow!("无法保存清除意图，请检查文件权限"))?;
    file.sync_all().map_err(|_| anyhow!("无法持久化清除意图"))?;
    // On Unix persist the new directory entry as well.
    #[cfg(unix)]
    if let Some(dir) = legacy.parent() {
        fs::File::open(dir).and_then(|file| file.sync_all())
            .map_err(|_| anyhow!("无法持久化凭据清除状态"))?;
    }
    Ok(())
}
fn load_from(store: &impl Store, legacy: &Path) -> Result<Option<Credentials>> {
    if marker_present(legacy)? {
        // Do not retry authentication with a record the user tried to erase.
        bail!("上次凭据清除未完成，请再次清除或重新设置凭据");
    }
    if let Some(value) = store.read()? {
        let secure = decode(&value)?;
        // Resume a partially completed migration only if both records agree.
        // A conflicting legacy record requires explicit user action.
        if let Some(old) = config::read_credentials(legacy)? {
            if old.account != secure.account || old.password != secure.password {
                bail!("新旧凭据冲突，请重新设置凭据或清除凭据");
            }
            remove_legacy(legacy)?;
        }
        return Ok(Some(secure));
    }
    if let Some(old) = config::read_credentials(legacy)? {
        write_verified(store, &old)?;
        remove_legacy(legacy)?;
        return Ok(Some(old));
    }
    Ok(None)
}

fn save_to(store: &impl Store, legacy: &Path, value: &Credentials) -> Result<()> {
    write_verified(store, value)?;
    remove_legacy(legacy)?;
    remove_legacy(&clear_marker(legacy))
}

fn clear_from(store: &impl Store, legacy: &Path) -> Result<()> {
    // Remove the legacy file first, otherwise a subsequent start could migrate
    // it back into the store after a successful keyring deletion.
    mark_clear(legacy)?;
    remove_legacy(legacy)?;
    store.delete()?;
    if store.read()?.is_some() {
        bail!("凭据删除校验失败，请重新清除");
    }
    remove_legacy(&clear_marker(legacy))
}

pub(crate) fn load(legacy: &Path) -> Result<Option<Credentials>> {
    load_from(&NativeStore::open()?, legacy)
}
pub(crate) fn save(legacy: &Path, value: &Credentials) -> Result<()> {
    save_to(&NativeStore::open()?, legacy, value)
}
pub(crate) fn clear(legacy: &Path) -> Result<()> {
    // Record user intent even if opening the native store itself fails.
    mark_clear(legacy)?;
    clear_from(&NativeStore::open()?, legacy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    #[derive(Default)]
    struct FakeStore {
        value: RefCell<Option<String>>,
        fail_write: Cell<bool>,
        wrong_readback: Cell<bool>,
        fail_delete: Cell<bool>,
    }
    impl Store for FakeStore {
        fn read(&self) -> Result<Option<String>> {
            Ok(self.value.borrow().clone())
        }
        fn write(&self, value: &str) -> Result<()> {
            if self.fail_write.get() { bail!("test store unavailable"); }
            *self.value.borrow_mut() = Some(if self.wrong_readback.get() {
                "{}".into()
            } else { value.into() });
            Ok(())
        }
        fn delete(&self) -> Result<()> {
            if self.fail_delete.get() { bail!("test delete failed"); }
            *self.value.borrow_mut() = None;
            Ok(())
        }
    }
    struct Temp(std::path::PathBuf);
    impl Temp {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "tyut-credential-test-{}-{}", std::process::id(), rand::random::<u64>()));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
        fn legacy(&self) -> std::path::PathBuf { self.0.join("config.enc") }
    }
    impl Drop for Temp {
        fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
    }
    fn credential(account: &str) -> Credentials {
        Credentials {
            account: config::xor_encode(account),
            password: config::xor_encode("test-password"),
            saved_at: None,
        }
    }
    #[test]
    fn migration_verifies_then_removes_legacy() {
        let temp = Temp::new();
        let legacy = temp.legacy();
        let old = credential("old-account");
        fs::write(&legacy, serde_json::to_string(&old).unwrap()).unwrap();
        let store = FakeStore::default();
        assert!(load_from(&store, &legacy).unwrap().as_ref() == Some(&old));
        assert!(!legacy.exists());
        assert!(load_from(&store, &legacy).unwrap().as_ref() == Some(&old));
    }
    #[test]
    fn failed_migration_preserves_original() {
        let temp = Temp::new();
        let legacy = temp.legacy();
        let raw = serde_json::to_string(&credential("old-account")).unwrap();
        fs::write(&legacy, &raw).unwrap();
        let store = FakeStore::default();
        store.fail_write.set(true);
        assert!(load_from(&store, &legacy).is_err());
        assert_eq!(fs::read_to_string(&legacy).unwrap(), raw);
        store.fail_write.set(false);
        store.wrong_readback.set(true);
        assert!(load_from(&store, &legacy).is_err());
        assert_eq!(fs::read_to_string(&legacy).unwrap(), raw);
    }
    #[test]
    fn interrupted_clear_blocks_load_until_explicit_retry() {
        let temp = Temp::new();
        let legacy = temp.legacy();
        let store = FakeStore::default();
        save_to(&store, &legacy, &credential("old-account")).unwrap();
        store.fail_delete.set(true);
        assert!(clear_from(&store, &legacy).is_err());
        assert!(marker_present(&legacy).unwrap());
        assert!(load_from(&store, &legacy).is_err());
        store.fail_delete.set(false);
        clear_from(&store, &legacy).unwrap();
        assert!(load_from(&store, &legacy).unwrap().is_none());
        assert!(!marker_present(&legacy).unwrap());
    }
    #[test]
    fn explicit_save_resolves_interrupted_clear() {
        let temp = Temp::new();
        let legacy = temp.legacy();
        let store = FakeStore::default();
        mark_clear(&legacy).unwrap();
        let new = credential("new-account");
        save_to(&store, &legacy, &new).unwrap();
        assert!(load_from(&store, &legacy).unwrap().as_ref() == Some(&new));
    }
    #[test]
    fn account_switch_replaces_record_and_clear_does_not_resurrect_it() {
        let temp = Temp::new();
        let legacy = temp.legacy();
        let store = FakeStore::default();
        let old = credential("old-account");
        let new = credential("new-account");
        fs::write(&legacy, serde_json::to_string(&old).unwrap()).unwrap();
        save_to(&store, &legacy, &new).unwrap();
        assert!(load_from(&store, &legacy).unwrap().as_ref() == Some(&new));
        clear_from(&store, &legacy).unwrap();
        assert!(load_from(&store, &legacy).unwrap().is_none());
    }
}
