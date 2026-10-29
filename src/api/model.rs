//! 数据模型与本地持久化。
//!
//! 持久化文件位于桌面 exe 目录或 Android 应用私有目录：accounts.json（多账号账号库，
//! 每个账号保存自己的设备身份与登录态）、config.json（全局跑步参数）、identity.json /
//! session.json（活动账号的兼容镜像）、points_cache.json（点位缓存）。
//! 同一账号的 device_id 固定复用（严禁每次随机，会触发 10121 风控）；不同账号使用不同身份。

pub use crate::crypto::header::HeaderIdentity;

pub const HOST: &str = "https://run.gxapp.iydsj.com";
/// 排行榜 / 违规名单域名（信封链与 RUN 相同）。
pub const DISCOVERY: &str = "https://discovery.gxapp.iydsj.com";

/// 登录态（session.json）。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Session {
    #[serde(default)]
    pub uid: i64,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub unid: String,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_weight")]
    pub weight: f64,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub device_id: String,
    /// 登录响应完整业务数据（我的页展示）
    #[serde(default)]
    pub profile: serde_json::Value,
}

fn default_weight() -> f64 {
    68.0
}

impl Session {
    pub fn is_logged_in(&self) -> bool {
        self.uid >= 1 && !self.token.is_empty()
    }

    /// 当前校区标识：优先校园资料中的 campusId，旧会话回退 unid。
    pub fn campus_id(&self) -> String {
        match self.profile.get("campusId") {
            Some(serde_json::Value::String(text)) if !text.trim().is_empty() => {
                text.trim().to_string()
            }
            Some(serde_json::Value::Number(number)) => number.to_string(),
            _ => self.unid.trim().to_string(),
        }
    }
}

/// 一个账号及其专属设备身份。用户名是账号库内唯一键。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Account {
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub remember: bool,
    #[serde(default)]
    pub identity: HeaderIdentity,
    #[serde(default)]
    pub session: Option<Session>,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub updated_at: i64,
}

impl Account {
    pub fn new(username: impl Into<String>, identity: HeaderIdentity) -> Self {
        Self {
            username: username.into(),
            password: String::new(),
            remember: false,
            identity,
            session: None,
            display_name: String::new(),
            updated_at: crate::crypto::envelope::now_ms(),
        }
    }

    pub fn is_blank_draft(&self) -> bool {
        self.username.trim().is_empty()
    }
}

/// accounts.json：`active` 为当前账号用户名；空用户名保留给尚未登录的新账号草稿。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct AccountStore {
    #[serde(default, alias = "active_username")]
    pub active: String,
    #[serde(default)]
    pub accounts: Vec<Account>,
}

impl AccountStore {
    pub fn find(&self, username: &str) -> Option<&Account> {
        let username = username.trim();
        self.accounts
            .iter()
            .find(|account| account.username.trim() == username)
    }

    pub fn find_mut(&mut self, username: &str) -> Option<&mut Account> {
        let username = username.trim();
        self.accounts
            .iter_mut()
            .find(|account| account.username.trim() == username)
    }

    pub fn active_account(&self) -> Option<&Account> {
        self.find(&self.active)
    }

    pub fn active_account_mut(&mut self) -> Option<&mut Account> {
        let active = self.active.clone();
        self.find_mut(&active)
    }

    pub fn upsert(&mut self, account: Account) {
        if let Some(existing) = self.find_mut(&account.username) {
            *existing = account;
        } else {
            self.accounts.push(account);
        }
    }

    pub fn ensure_active(&mut self) -> Option<&Account> {
        if self.active_account().is_none() {
            if let Some(first) = self.accounts.first() {
                self.active = first.username.clone();
            }
        }
        self.active_account()
    }
}

/// UI 参数（config.json）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Config {
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub remember: bool,
    #[serde(default = "default_f32")]
    pub dist_min: f32,
    #[serde(default = "default_f32")]
    pub dist_max: f32,
    #[serde(default = "default_pace_min")]
    pub pace_min: f32,
    #[serde(default = "default_pace_max")]
    pub pace_max: f32,
    #[serde(default)]
    pub face_check: bool,
    /// 跑步时覆盖轨迹点 bdA；为空表示使用生成器的海拔曲线。
    #[serde(default)]
    pub manual_altitude: Option<f64>,
    /// 跑步时将生成器海拔曲线映射到此范围；为空表示不使用范围覆盖。
    #[serde(default)]
    pub manual_altitude_range: Option<crate::track::altitude::AltitudeRange>,
    #[serde(default = "default_ai_minutes")]
    pub ai_minutes: i64,
    #[serde(default = "default_ai_reps")]
    pub ai_reps: i64,
    /// 启动时检查更新：silent（静默，发现新版才弹窗）/ ask（每次询问）/ off
    #[serde(default = "default_update_check")]
    pub update_check: String,
    /// OSM 路网文件路径（真实道路路由用）。
    #[serde(default)]
    pub osm_path: String,
    /// 路线算法：legacy | road。
    #[serde(default = "default_route_mode")]
    pub route_mode: String,
}

fn default_route_mode() -> String {
    "legacy".into()
}

fn default_f32() -> f32 {
    1.0
}
fn default_pace_min() -> f32 {
    360.0
}
fn default_pace_max() -> f32 {
    480.0
}
fn default_ai_minutes() -> i64 {
    1
}
fn default_ai_reps() -> i64 {
    5
}
fn default_update_check() -> String {
    "silent".into()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            username: String::new(),
            password: String::new(),
            remember: false,
            dist_min: 1.0,
            dist_max: 1.5,
            pace_min: default_pace_min(),
            pace_max: default_pace_max(),
            face_check: true,
            manual_altitude: None,
            manual_altitude_range: None,
            ai_minutes: default_ai_minutes(),
            ai_reps: default_ai_reps(),
            update_check: default_update_check(),
            osm_path: String::new(),
            route_mode: default_route_mode(),
        }
    }
}

// ── 持久化（平台数据目录）──────────────────────────────────────

fn exe_dir() -> std::path::PathBuf {
    crate::platform::data_dir()
}

fn read_json<T: serde::de::DeserializeOwned>(name: &str) -> Option<T> {
    let path = exe_dir().join(name);
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_json<T: serde::Serialize>(name: &str, value: &T) -> Result<(), String> {
    let path = exe_dir().join(name);
    let json =
        serde_json::to_string_pretty(value).map_err(|e| format!("序列化 {name} 失败: {e}"))?;
    std::fs::write(&path, json).map_err(|e| format!("写入 {} 失败: {e}", path.display()))
}

fn current_platform() -> &'static str {
    if cfg!(target_os = "android") {
        "android"
    } else {
        "ios"
    }
}

fn initialize_identity(id: &mut HeaderIdentity) -> bool {
    let mut dirty = false;
    if id.device_id.is_empty() {
        id.device_id = uuid::Uuid::new_v4().to_string().to_uppercase();
        dirty = true;
    }
    if id.app_install_time <= 0 {
        id.app_install_time = HeaderIdentity::fresh_install_time(&id.platform);
        dirty = true;
    }
    if id.mac_address.is_empty() {
        id.mac_address = crate::crypto::header::random_mac();
        dirty = true;
    }
    dirty
}

/// 新账号专用身份：每一次调用都生成全新 DeviceId / MAC / 安装时间。
pub fn new_identity_for_platform(platform: &str) -> HeaderIdentity {
    let mut identity = HeaderIdentity::default();
    if platform == "android" {
        identity.platform = "android".into();
        identity.device_name = "Android".into();
        identity.os_version = "16".into();
    }
    initialize_identity(&mut identity);
    identity
}

/// 读取旧版单账号 identity.json；迁移前和兼容测试使用，不读取账号库。
fn load_legacy_identity_for_platform(platform: &str) -> HeaderIdentity {
    let mut id: HeaderIdentity = if let Some(id) = read_json("identity.json") {
        id
    } else {
        let id = new_identity_for_platform(platform);
        let _ = write_json("identity.json", &id);
        return id;
    };
    if initialize_identity(&mut id) {
        let _ = write_json("identity.json", &id);
    }
    id
}

/// 加载活动账号的设备身份；首次运行自动从旧版文件迁移。
pub fn load_identity() -> HeaderIdentity {
    active_account()
        .map(|account| account.identity)
        .unwrap_or_else(|| load_legacy_identity_for_platform(current_platform()))
}

fn mirror_active_account(store: &AccountStore) {
    let Some(account) = store.active_account() else {
        return;
    };
    let _ = write_json("identity.json", &account.identity);
    match &account.session {
        Some(session) if session.is_logged_in() => {
            let _ = write_json("session.json", session);
        }
        _ => {
            let _ = std::fs::remove_file(exe_dir().join("session.json"));
        }
    }
}

fn migrate_legacy_accounts() -> AccountStore {
    let cfg: Config = read_json("config.json").unwrap_or_default();
    let legacy_session: Session = read_json("session.json").unwrap_or_default();
    let username = if !cfg.username.trim().is_empty() {
        cfg.username.trim().to_string()
    } else {
        legacy_session.username.trim().to_string()
    };
    let mut identity = load_legacy_identity_for_platform(current_platform());
    let _ = initialize_identity(&mut identity);
    let mut account = Account::new(username.clone(), identity);
    account.remember = cfg.remember;
    if cfg.remember {
        account.password = cfg.password.clone();
    }
    account.display_name = legacy_session.name.clone();
    if legacy_session.is_logged_in() {
        account.session = Some(legacy_session);
    }
    AccountStore {
        active: username,
        accounts: vec![account],
    }
}

/// 加载账号库。旧版 config/identity/session 只在 accounts.json 不存在时迁移一次。
pub fn load_accounts() -> AccountStore {
    let mut store: AccountStore = read_json("accounts.json").unwrap_or_default();
    let mut dirty = false;
    if store.accounts.is_empty() {
        store = migrate_legacy_accounts();
        dirty = true;
    }
    for account in &mut store.accounts {
        dirty |= initialize_identity(&mut account.identity);
    }
    let previous_active = store.active.clone();
    if store.ensure_active().is_none() {
        let account = Account::new(String::new(), new_identity_for_platform(current_platform()));
        store.active = account.username.clone();
        store.accounts.push(account);
        dirty = true;
    }
    dirty |= store.active != previous_active;
    if dirty {
        let _ = save_accounts(&store);
    }
    store
}

pub fn save_accounts(store: &AccountStore) -> Result<(), String> {
    write_json("accounts.json", store)
}

pub fn active_account() -> Option<Account> {
    load_accounts().active_account().cloned()
}

pub fn create_draft_account() -> Result<Account, String> {
    let mut store = load_accounts();
    if let Some(blank) = store
        .accounts
        .iter()
        .find(|account| account.is_blank_draft())
    {
        let account = blank.clone();
        store.active = account.username.clone();
        save_accounts(&store)?;
        mirror_active_account(&store);
        return Ok(account);
    }
    let account = Account::new(String::new(), new_identity_for_platform(current_platform()));
    store.active = account.username.clone();
    store.accounts.push(account.clone());
    save_accounts(&store)?;
    mirror_active_account(&store);
    Ok(account)
}

/// 登录前准备账号：已有用户名沿用原设备；新用户名创建独立设备；空白草稿会被正式命名。
pub fn prepare_login_account(
    username: &str,
    password: &str,
    remember: bool,
) -> Result<Account, String> {
    let username = username.trim();
    if username.is_empty() {
        return Err("账号不能为空".into());
    }
    let mut store = load_accounts();
    let mut account = if let Some(existing) = store.find(username) {
        existing.clone()
    } else {
        let identity = store
            .accounts
            .iter()
            .position(Account::is_blank_draft)
            .map(|index| store.accounts.remove(index).identity)
            .unwrap_or_else(|| new_identity_for_platform(current_platform()));
        Account::new(username.to_string(), identity)
    };
    account.username = username.to_string();
    account.password = if remember {
        password.to_string()
    } else {
        String::new()
    };
    account.remember = remember;
    account.updated_at = crate::crypto::envelope::now_ms();
    store.upsert(account.clone());
    store.active = username.to_string();
    save_accounts(&store)?;
    mirror_active_account(&store);
    Ok(account)
}

pub fn switch_account(username: &str) -> Result<Account, String> {
    let username = username.trim();
    let mut store = load_accounts();
    let account = store
        .find(username)
        .cloned()
        .ok_or_else(|| format!("账号不存在: {username}"))?;
    store.active = username.to_string();
    save_accounts(&store)?;
    mirror_active_account(&store);
    Ok(account)
}

pub fn remove_account(username: &str) -> Result<AccountStore, String> {
    let username = username.trim();
    let mut store = load_accounts();
    let before = store.accounts.len();
    store
        .accounts
        .retain(|account| account.username.trim() != username);
    if store.accounts.len() == before {
        return Err(format!("账号不存在: {username}"));
    }
    if store.accounts.is_empty() {
        store.accounts.push(Account::new(
            String::new(),
            new_identity_for_platform(current_platform()),
        ));
    }
    store.active = store.accounts[0].username.clone();
    save_accounts(&store)?;
    mirror_active_account(&store);
    Ok(store)
}

pub fn save_identity_for(username: &str, identity: &HeaderIdentity) -> Result<(), String> {
    let username = username.trim();
    let mut store = load_accounts();
    let mut identity = identity.clone();
    initialize_identity(&mut identity);
    let account = store
        .find_mut(username)
        .ok_or_else(|| format!("账号不存在: {username}"))?;
    account.identity = identity;
    account.updated_at = crate::crypto::envelope::now_ms();
    save_accounts(&store)?;
    if store.active.trim() == username {
        mirror_active_account(&store);
    }
    Ok(())
}

pub fn save_session_for(username: &str, session: &Session) -> Result<(), String> {
    let username = username.trim();
    let mut store = load_accounts();
    let account = store
        .find_mut(username)
        .ok_or_else(|| format!("账号不存在: {username}"))?;
    account.session = session.is_logged_in().then_some(session.clone());
    account.display_name = session.name.clone();
    account.updated_at = crate::crypto::envelope::now_ms();
    save_accounts(&store)?;
    if store.active.trim() == username {
        mirror_active_account(&store);
    }
    Ok(())
}

/// 登录成功时同时写入该账号专属身份和会话，避免账号库里的 DeviceId 与会话不一致。
pub fn save_login_state_for(
    username: &str,
    identity: &HeaderIdentity,
    session: &Session,
) -> Result<(), String> {
    let username = username.trim();
    let mut store = load_accounts();
    let mut identity = identity.clone();
    initialize_identity(&mut identity);
    let mut account = store
        .find(username)
        .cloned()
        .unwrap_or_else(|| Account::new(username.to_string(), identity.clone()));
    account.username = username.to_string();
    account.identity = identity;
    account.session = session.is_logged_in().then_some(session.clone());
    account.display_name = session.name.clone();
    account.updated_at = crate::crypto::envelope::now_ms();
    store.upsert(account);
    store.active = username.to_string();
    save_accounts(&store)?;
    mirror_active_account(&store);
    Ok(())
}

pub fn clear_session_for(username: &str) {
    let username = username.trim();
    let mut store = load_accounts();
    if let Some(account) = store.find_mut(username) {
        account.session = None;
        account.updated_at = crate::crypto::envelope::now_ms();
        let _ = save_accounts(&store);
        if store.active.trim() == username {
            mirror_active_account(&store);
        }
    }
}

/// 免密登录凭据：仅当账号勾选了“记住密码”且确有保存密码时返回。
pub fn saved_password(username: &str) -> Option<String> {
    let store = load_accounts();
    let account = store.find(username)?;
    (account.remember && !account.password.is_empty()).then(|| account.password.clone())
}

#[cfg(any(target_os = "android", test))]
#[derive(Debug, serde::Deserialize)]
pub struct DeviceInfo {
    pub manufacturer: String,
    pub model: String,
    pub os_version: String,
}

#[cfg(any(target_os = "android", test))]
pub fn identity_with_device_info(
    identity: &HeaderIdentity,
    info: &DeviceInfo,
) -> Result<HeaderIdentity, String> {
    if info.model.trim().is_empty() || info.os_version.trim().is_empty() {
        return Err("未能读取完整的机型和系统版本，请手动填写".into());
    }
    let mut updated = identity.clone();
    updated.platform = "android".into();
    updated.device_name = info.model.trim().into();
    updated.manufacturer = info.manufacturer.trim().into();
    updated.os_version = info.os_version.trim().into();
    Ok(updated)
}

pub fn save_identity(id: &HeaderIdentity) -> Result<(), String> {
    let username = load_accounts().active;
    save_identity_for(&username, id)
}

/// AI 项目列表永久缓存：拉取成功即落盘，网络异常时兜底展示。
pub fn load_ai_sports() -> Option<Vec<crate::api::ai::AiSport>> {
    read_json("ai_sports.json")
}

pub fn save_ai_sports(list: &[crate::api::ai::AiSport]) -> Result<(), String> {
    write_json("ai_sports.json", &list.to_vec())
}

pub fn load_session() -> Session {
    active_account()
        .and_then(|account| account.session)
        .unwrap_or_default()
}

pub fn save_session(s: &Session) -> Result<(), String> {
    let username = if s.username.trim().is_empty() {
        load_accounts().active
    } else {
        s.username.trim().to_string()
    };
    save_session_for(&username, s)
}

pub fn clear_session() {
    let username = load_accounts().active;
    clear_session_for(&username);
}

/// 用本机保存的密码重新登录当前活动账号，并把新会话写回账号库。
///
/// 服务端单会话：手机 App 一登录就会把本机 token 顶掉，表现为 HTTP 401
/// 「token有误或缓存中token不存在」。提交前用这个函数静默续期。
pub fn relogin_active(log: &mut dyn FnMut(&str)) -> Result<Session, String> {
    let store = load_accounts();
    let account = store
        .accounts
        .iter()
        .find(|a| a.username == store.active)
        .or_else(|| store.accounts.first())
        .cloned()
        .ok_or_else(|| "本机没有保存任何账号".to_string())?;
    if account.password.trim().is_empty() {
        return Err("本机未保存密码，请手动重新登录".into());
    }
    let mut client = crate::api::client::ApiClient::new(account.identity.clone(), None);
    let session = crate::api::login::login(&mut client, &account.username, &account.password, log)?;
    save_session_for(&account.username, &session)?;
    Ok(session)
}

pub fn load_config() -> Config {
    read_json("config.json").unwrap_or_default()
}

pub fn save_config(c: &Config) -> Result<(), String> {
    write_json("config.json", c)
}

#[cfg(test)]
mod storage_tests {
    use super::*;

    #[test]
    fn android_identity_import_persists_brand_and_reuses_uuid() {
        let directory =
            std::env::temp_dir().join(format!("xierun-identity-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        crate::platform::TEST_DATA_DIR.with(|p| *p.borrow_mut() = Some(directory.clone()));
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                crate::platform::TEST_DATA_DIR.with(|p| *p.borrow_mut() = None);
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(directory);
        let first = load_legacy_identity_for_platform("android");
        assert_eq!(first.platform, "android");
        assert_ne!(first.device_name, "iPhone");
        assert!(uuid::Uuid::parse_str(&first.device_id).is_ok());
        let again = load_legacy_identity_for_platform("android");
        assert_eq!(
            serde_json::to_value(&first).unwrap(),
            serde_json::to_value(&again).unwrap()
        );

        let mut existing = first.clone();
        existing.platform = "ios".into();
        existing.device_name = "Existing manual name".into();
        existing.idfa = "manual-value".into();
        let mut legacy = serde_json::to_value(&existing).unwrap();
        legacy.as_object_mut().unwrap().remove("manufacturer");
        write_json("identity.json", &legacy).unwrap();
        let loaded = load_legacy_identity_for_platform("android");
        assert_eq!(
            serde_json::to_value(&loaded).unwrap(),
            serde_json::to_value(&existing).unwrap(),
            "opening an existing install must not overwrite a manual identity"
        );

        let info: DeviceInfo = serde_json::from_str(
            r#"{"manufacturer":"Example","model":"Phone 16","os_version":"16"}"#,
        )
        .unwrap();
        assert_eq!(info.manufacturer, "Example");
        let imported = identity_with_device_info(&loaded, &info).unwrap();
        assert_eq!(imported.platform, "android");
        assert_eq!(imported.device_name, "Phone 16");
        assert_eq!(imported.os_version, "16");
        let mut expected = serde_json::to_value(&existing).unwrap();
        expected["platform"] = "android".into();
        expected["device_name"] = "Phone 16".into();
        expected["manufacturer"] = "Example".into();
        expected["os_version"] = "16".into();
        assert_eq!(
            serde_json::to_value(&imported).unwrap(),
            expected,
            "import must preserve UUID, MAC, manual IMEI/IDFA, install time and location"
        );
        assert_eq!(
            load_identity().device_name,
            existing.device_name,
            "preview must not save implicitly"
        );
        save_identity(&imported).unwrap();
        let stored: serde_json::Value = read_json("identity.json").unwrap();
        assert_eq!(
            stored.get("manufacturer").and_then(|value| value.as_str()),
            Some("Example"),
            "the consented brand must be written to identity.json"
        );
        let reloaded = load_identity();
        assert_eq!(
            serde_json::to_value(&reloaded).unwrap(),
            serde_json::to_value(&imported).unwrap(),
            "brand and identity must survive a fresh load from disk"
        );
        assert_eq!(reloaded.device_id, first.device_id);
        let (header, _) =
            crate::crypto::header::build_header_for(&reloaded, 0, "", Some(1_700_000_000_000));
        let header: serde_json::Value = serde_json::from_str(&header).unwrap();
        assert_eq!(header["DeviceId"], first.device_id);
        assert_eq!(
            header["deviceName"], "Phone 16",
            "brand must not be prepended to the protocol model field"
        );
        assert_eq!(header["osVersion"], "16");
        assert!(
            header.get("manufacturer").is_none() && header.get("brand").is_none(),
            "persisting local brand metadata must not invent new protocol fields"
        );
    }

    #[test]
    fn incomplete_native_device_info_cannot_replace_identity() {
        let identity = HeaderIdentity::default();
        for (model, os_version) in [("", "16"), ("Phone", " ")] {
            let info = DeviceInfo {
                manufacturer: String::new(),
                model: model.into(),
                os_version: os_version.into(),
            };
            assert!(identity_with_device_info(&identity, &info).is_err());
        }
    }

    #[test]
    fn session_roundtrip_and_logout_use_private_data_directory() {
        let directory = std::env::temp_dir().join(format!("xierun-storage-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        crate::platform::TEST_DATA_DIR.with(|p| *p.borrow_mut() = Some(directory.clone()));
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                crate::platform::TEST_DATA_DIR.with(|p| *p.borrow_mut() = None);
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(directory.clone());
        assert_eq!(
            exe_dir(),
            directory,
            "storage must use the injected app directory"
        );
        let session = Session {
            uid: 123,
            token: "test-only-token".into(),
            ..Default::default()
        };
        save_session(&session).unwrap();
        assert!(directory.join("session.json").is_file());
        assert_eq!(load_session().uid, 123);
        assert_eq!(load_session().token, session.token);
        clear_session();
        assert!(!directory.join("session.json").exists());
        assert_eq!(load_session().uid, 0);
    }

    #[test]
    fn account_store_isolates_devices_and_restores_each_session() {
        let directory =
            std::env::temp_dir().join(format!("xierun-accounts-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        crate::platform::TEST_DATA_DIR.with(|p| *p.borrow_mut() = Some(directory.clone()));
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                crate::platform::TEST_DATA_DIR.with(|p| *p.borrow_mut() = None);
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(directory);

        let draft = load_accounts();
        assert_eq!(draft.accounts.len(), 1);
        assert!(draft.accounts[0].is_blank_draft());
        let draft_device = draft.accounts[0].identity.device_id.clone();

        let first = prepare_login_account("10001", "pw1", true).unwrap();
        assert_eq!(first.identity.device_id, draft_device);
        let first_session = Session {
            uid: 101,
            token: "token-1".into(),
            username: first.username.clone(),
            device_id: first.identity.device_id.clone(),
            ..Default::default()
        };
        save_login_state_for(&first.username, &first.identity, &first_session).unwrap();

        let second = prepare_login_account("10002", "pw2", false).unwrap();
        assert_ne!(first.identity.device_id, second.identity.device_id);
        assert_ne!(first.identity.mac_address, second.identity.mac_address);
        let second_session = Session {
            uid: 202,
            token: "token-2".into(),
            username: second.username.clone(),
            device_id: second.identity.device_id.clone(),
            ..Default::default()
        };
        save_login_state_for(&second.username, &second.identity, &second_session).unwrap();

        switch_account("10001").unwrap();
        assert_eq!(load_identity().device_id, first.identity.device_id);
        assert_eq!(load_session().uid, 101);
        switch_account("10002").unwrap();
        assert_eq!(load_identity().device_id, second.identity.device_id);
        assert_eq!(load_session().uid, 202);

        let store = load_accounts();
        let first = store.find("10001").unwrap();
        assert!(first.remember);
        assert_eq!(first.password, "pw1");
        let second = store.find("10002").unwrap();
        assert!(!second.remember);
        assert!(second.password.is_empty());

        // 只有勾选“记住密码”的账号才提供免密登录凭据。
        assert_eq!(saved_password("10001").as_deref(), Some("pw1"));
        assert_eq!(saved_password("10002"), None);

        let store = remove_account("10002").unwrap();
        assert_eq!(store.active, "10001");
        assert_eq!(load_session().uid, 101);
        let store = remove_account("10001").unwrap();
        assert_eq!(store.accounts.len(), 1);
        assert!(store.accounts[0].is_blank_draft());
    }

    #[test]
    fn legacy_single_account_is_migrated_only_once() {
        let directory = std::env::temp_dir().join(format!("xierun-migrate-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        crate::platform::TEST_DATA_DIR.with(|p| *p.borrow_mut() = Some(directory.clone()));
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                crate::platform::TEST_DATA_DIR.with(|p| *p.borrow_mut() = None);
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(directory);

        let mut legacy_identity = HeaderIdentity::default();
        legacy_identity.device_id = "LEGACY-DEVICE".into();
        write_json("identity.json", &legacy_identity).unwrap();
        write_json(
            "config.json",
            &Config {
                username: "legacy-user".into(),
                password: "legacy-password".into(),
                remember: true,
                ..Default::default()
            },
        )
        .unwrap();
        write_json(
            "session.json",
            &Session {
                uid: 7,
                token: "legacy-token".into(),
                username: "legacy-user".into(),
                ..Default::default()
            },
        )
        .unwrap();

        let migrated = load_accounts();
        assert_eq!(migrated.active, "legacy-user");
        assert_eq!(migrated.accounts.len(), 1);
        assert_eq!(migrated.accounts[0].identity.device_id, "LEGACY-DEVICE");
        assert_eq!(migrated.accounts[0].password, "legacy-password");
        assert_eq!(migrated.accounts[0].session.as_ref().unwrap().uid, 7);

        write_json("identity.json", &new_identity_for_platform("ios")).unwrap();
        write_json(
            "config.json",
            &Config {
                username: "replacement-user".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let reloaded = load_accounts();
        assert_eq!(reloaded.active, "legacy-user");
        assert_eq!(reloaded.accounts.len(), 1);
        assert_eq!(reloaded.accounts[0].identity.device_id, "LEGACY-DEVICE");
    }
}

/// 点位缓存：{ts_ms, points}，TTL 300s（服务端限流 10603：5 分钟 3 次）。
pub const POINTS_TTL_MS: i64 = 300_000;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct PointsCache {
    ts: i64,
    anchor: crate::location::Coordinate,
    points: Vec<serde_json::Value>,
}

/// 只返回与本次锚点相同的缓存。旧版未记录锚点的缓存会自然失效，避免串城市。
pub fn load_points_cache_for(
    anchor: crate::location::Coordinate,
) -> Option<(i64, Vec<serde_json::Value>)> {
    let v: serde_json::Value = read_json("points_cache.json")?;
    let cache: PointsCache = serde_json::from_value(v).ok()?;
    if !cache.anchor.is_near(anchor, 0.0001) {
        return None;
    }
    let ts = cache.ts;
    let pts = cache.points;
    Some((ts, pts))
}

pub fn save_points_cache(
    anchor: crate::location::Coordinate,
    points: &[serde_json::Value],
) -> Result<(), String> {
    let doc = PointsCache {
        ts: crate::crypto::envelope::now_ms(),
        anchor,
        points: points.to_vec(),
    };
    write_json("points_cache.json", &doc)
}

#[cfg(test)]
mod points_cache_tests {
    use super::*;
    #[test]
    fn cache_is_scoped_to_anchor() {
        let cache = PointsCache {
            ts: 1,
            anchor: crate::location::Coordinate::new(39.9, 116.4, 0.0).unwrap(),
            points: vec![],
        };
        let decoded: PointsCache =
            serde_json::from_value(serde_json::to_value(cache).unwrap()).unwrap();
        assert!(decoded.anchor.is_near(
            crate::location::Coordinate::new(39.9, 116.4, 0.0).unwrap(),
            0.0001
        ));
        assert!(!decoded.anchor.is_near(
            crate::location::Coordinate::new(38.9, 121.5, 0.0).unwrap(),
            0.0001
        ));
    }
}

/// 围栏缓存：拉取成功后落盘，UI 预览无网时兜底。
pub fn load_fence_cache() -> Option<Vec<Vec<(f64, f64)>>> {
    load_fence_cache_with_ts().map(|(_ts, fences)| fences)
}

/// 带时间戳的围栏缓存：提交前判断「预览用的那份围栏是否还能复用」。
///
/// 预览用缓存、提交重新拉取会让两边的围栏集合不同 → 路线跟着不同（预览与提交不一致）。
/// 因此提交优先复用新鲜缓存（含空围栏：空也是一种有效结果）。
pub fn load_fence_cache_with_ts() -> Option<(i64, Vec<Vec<(f64, f64)>>)> {
    let v: serde_json::Value = read_json("fence_cache.json")?;
    let fences = serde_json::from_value(v.get("fences")?.clone()).ok()?;
    let ts = v.get("ts").and_then(|t| t.as_i64()).unwrap_or(0);
    Some((ts, fences))
}

/// 围栏缓存有效期：围栏极少变动，24 小时内直接复用（保证提交与预览同源）。
pub const FENCE_TTL_MS: i64 = 24 * 60 * 60 * 1000;

pub fn save_fence_cache(fences: &[Vec<(f64, f64)>]) -> Result<(), String> {
    let doc = serde_json::json!({
        "ts": crate::crypto::envelope::now_ms(),
        "fences": fences,
    });
    write_json("fence_cache.json", &doc)
}

/// 读取 `checkpoints.json`（点库，BD-09）为 (lat, lon) 列表。
///
/// 用途：`points_cache.json` 有过期/清空的情况，点库是长期积累的，
/// 预览和成线都应该优先还能用上它。
pub fn load_checkpoints_bd() -> Vec<(f64, f64)> {
    let v: serde_json::Value = match read_json("checkpoints.json") {
        Some(v) => v,
        None => return Vec::new(),
    };
    let mut out = Vec::new();
    let mut push = |la: Option<f64>, lo: Option<f64>| {
        if let (Some(la), Some(lo)) = (la, lo) {
            if la.abs() > 1.0 && lo.abs() > 1.0 {
                out.push((la, lo));
            }
        }
    };
    if let Some(arr) = v.get("points").and_then(|p| p.as_array()) {
        for p in arr {
            push(
                p.get("lat").and_then(|x| x.as_f64()),
                p.get("lon").and_then(|x| x.as_f64()),
            );
        }
    }
    out
}

impl Config {
    /// 实际可用的路网路径：配置为空或文件不存在时，回退到目录内的 route.osm / campus.osm。
    pub fn effective_osm_path(&self) -> String {
        if !self.osm_path.is_empty() && std::path::Path::new(&self.osm_path).exists() {
            return self.osm_path.clone();
        }
        let dir = crate::platform::data_dir();
        for name in ["route.osm", "campus.osm"] {
            let p = dir.join(name);
            if p.exists() {
                return p.to_string_lossy().to_string();
            }
        }
        String::new()
    }
}