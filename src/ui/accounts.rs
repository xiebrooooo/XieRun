//! 多账号切换与账号级 UI 状态刷新。

use super::{ai, data, records, theme, user, App};
use crate::api::model::{self, Account};
use eframe::egui;

enum AccountAction {
    Switch(String),
    New,
    Delete(String),
}

impl App {
    pub(crate) fn active_account_username(&self) -> String {
        self.accounts.active.trim().to_string()
    }

    pub(crate) fn draw_account_bar(&mut self, ui: &mut egui::Ui) {
        let accounts = self.accounts.accounts.clone();
        let active = self.accounts.active.clone();
        let selected_text = accounts
            .iter()
            .find(|account| account.username == active)
            .map(account_label)
            .unwrap_or_else(|| "选择账号".to_string());
        let busy = self.any_busy();
        let mut action = None;

        ui.add_enabled_ui(!busy, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("账号库：");
                egui::ComboBox::from_id_salt("account_store_combo")
                    .width(210.0)
                    .selected_text(selected_text)
                    .show_ui(ui, |ui| {
                        for account in &accounts {
                            let selected = account.username == active;
                            if ui
                                .selectable_label(selected, account_label(account))
                                .clicked()
                            {
                                action = Some(AccountAction::Switch(account.username.clone()));
                            }
                        }
                        ui.separator();
                        if ui.button("＋ 新增账号").clicked() {
                            action = Some(AccountAction::New);
                        }
                    });
                if ui.small_button("新增").clicked() {
                    action = Some(AccountAction::New);
                }
                if !active.is_empty() && ui.small_button("删除").clicked() {
                    action = Some(AccountAction::Delete(active.clone()));
                }
                ui.colored_label(
                    theme::text_dim(),
                    format!("共 {} 个，一号一设备", accounts.len()),
                );
            });
        });

        match action {
            Some(AccountAction::Switch(username)) => self.switch_account_to(&username),
            Some(AccountAction::New) => self.begin_new_account(),
            Some(AccountAction::Delete(username)) => self.delete_account(&username),
            None => {}
        }
    }

    /// 将当前输入框中的密码/记住设置回写活动账号。空用户名草稿不落盘密码。
    pub(crate) fn persist_active_account_fields(&mut self) {
        let username = self.active_account_username();
        if username.is_empty() {
            return;
        }
        let mut store = model::load_accounts();
        if let Some(account) = store.find_mut(&username) {
            account.remember = self.remember;
            account.password = if self.remember {
                self.password.clone()
            } else {
                String::new()
            };
            account.updated_at = crate::crypto::envelope::now_ms();
            let _ = model::save_accounts(&store);
        }
    }

    pub(crate) fn switch_account_to(&mut self, username: &str) {
        if self.any_busy() {
            self.status = "当前有任务进行中，暂不能切换账号".into();
            return;
        }
        // 再次点击当前账号也允许重试免密登录，便于会话过期后直接恢复。
        if username == self.active_account_username() {
            self.try_silent_login();
            return;
        }
        self.persist_active_account_fields();
        match model::switch_account(username) {
            Ok(account) => {
                self.accounts = model::load_accounts();
                self.reload_account_state(account);
                self.status = format!("已切换账号：{}", self.username);
                self.refresh_active_account();
                self.try_silent_login();
            }
            Err(error) => self.status = format!("× 切换账号失败：{error}"),
        }
    }

    /// 会话失效但账号已保存密码时，自动使用已保存凭据免密登录。
    pub(crate) fn try_silent_login(&mut self) {
        if self.login_busy || self.session.is_some() || self.username.trim().is_empty() {
            return;
        }
        if self.password.is_empty() {
            let Some(saved) = model::saved_password(&self.username) else {
                return;
            };
            self.password = saved;
            self.remember = true;
        }
        self.do_login();
    }

    pub(crate) fn begin_new_account(&mut self) {
        if self.any_busy() {
            self.status = "当前有任务进行中，暂不能新增账号".into();
            return;
        }
        self.persist_active_account_fields();
        match model::create_draft_account() {
            Ok(account) => {
                self.accounts = model::load_accounts();
                self.reload_account_state(account);
                self.username.clear();
                self.password.clear();
                self.remember = false;
                self.status = "已创建独立设备身份，请填写账号并登录".into();
            }
            Err(error) => self.status = format!("× 新增账号失败：{error}"),
        }
    }

    pub(crate) fn delete_account(&mut self, username: &str) {
        if self.any_busy() {
            self.status = "当前有任务进行中，暂不能删除账号".into();
            return;
        }
        match model::remove_account(username) {
            Ok(store) => {
                self.accounts = store;
                if let Some(account) = self.accounts.active_account().cloned() {
                    self.reload_account_state(account);
                }
                self.status = format!("已删除账号：{}", username);
                self.refresh_active_account();
                self.try_silent_login();
            }
            Err(error) => self.status = format!("× 删除账号失败：{error}"),
        }
    }

    fn reload_account_state(&mut self, account: Account) {
        let is_blank = account.is_blank_draft();
        self.identity = account.identity.clone();
        self.device_buf = account.identity;
        self.session = account.session.filter(|session| session.is_logged_in());
        self.username = account.username.clone();
        self.password = if account.remember {
            account.password.clone()
        } else {
            String::new()
        };
        self.remember = account.remember;
        self.new_account_mode = is_blank && self.session.is_none();

        let ai_list = std::mem::take(&mut self.ai_page.list);
        self.ai_page = ai::AiPage::default();
        self.ai_page.list = ai_list;
        self.records_page = records::RecordsPage::default();
        self.data_page = data::DataPage::default();
        self.user_page = user::UserPage::default();
        self.run_page.preview = None;
        self.run_page.preview_stale = true;
        self.run_page.map_fitted = false;
        self.ai_confirm = None;
        self.popup = None;
    }

    fn refresh_active_account(&mut self) {
        if self.session.is_some() {
            self.refresh_data_page();
            self.refresh_records();
            self.refresh_user_page();
            self.refresh_ai_list();
            self.refresh_fence();
            self.refresh_points();
            self.start_school_location_detection(true);
        }
    }
}

fn account_label(account: &Account) -> String {
    let username = account.username.trim();
    if username.is_empty() {
        return "＋ 新账号（未登录）".into();
    }
    let name = account.display_name.trim();
    if !name.is_empty() && name != username {
        format!("{name}（{username}）")
    } else {
        username.to_string()
    }
}
