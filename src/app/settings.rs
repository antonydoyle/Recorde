//! Settings window: navigation shell and the General, Appearance, Git, Tools,
//! Keyboard, Repository, Updates and About pages. Accounts & SSH lives in `accounts.rs`.
#![allow(unused_imports)]
use super::*;
use crate::icons;
use crate::ui::{kit, motion};

fn git_readiness_status(ui: &mut egui::Ui, monitor: &mut crate::git::readiness::Monitor) {
    if monitor.checking() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Checking system Git…");
        });
    }
    if let Some(value) = &monitor.value {
        let status = if value.problem.is_some() {
            kit::Status::Error
        } else {
            kit::Status::Ok
        };
        kit::callout(ui, status, &value.summary());
        if let Some(path) = &value.executable {
            ui.label(format!("System Git executable: {}", path.display()));
        }
        ui.label(
            value
                .lfs_version
                .as_deref()
                .or(value.lfs_problem.as_deref())
                .unwrap_or("Git LFS was not checked because Git could not run."),
        );
    }
    let (major, minor, patch) = git2::Version::get().libgit2_version();
    kit::hint(ui, &format!("Bundled libgit2 {major}.{minor}.{patch} reads repositories independently of system Git. Recorde does not bundle Git."));
    if ui.add(kit::secondary_button("Retry Git check")).clicked() {
        monitor.retry();
    }
}

fn settings_categories() -> [(SettingsTab, char, &'static str, &'static str); 9] {
    [
        (
            SettingsTab::App,
            icons::SETTINGS,
            "General",
            "startup privacy pictures",
        ),
        (
            SettingsTab::Ui,
            icons::PALETTE,
            "Appearance",
            "theme palette scale contrast motion colour",
        ),
        (
            SettingsTab::Accounts,
            icons::VPN_KEY,
            "Accounts & SSH",
            "account authentication ssh key",
        ),
        (
            SettingsTab::Git,
            icons::ACCOUNT_TREE,
            "Git",
            "pull rebase merge fetch prune protected sign-off",
        ),
        (
            SettingsTab::Tools,
            icons::CONSTRUCTION,
            "Tools",
            "editor terminal diff merge",
        ),
        (
            SettingsTab::Shortcuts,
            icons::KEYBOARD,
            "Keyboard",
            "shortcuts keys commands",
        ),
        (
            SettingsTab::Repository,
            icons::FOLDER,
            "Repository",
            "remote identity recovery snapshots maintenance",
        ),
        (
            SettingsTab::Updates,
            icons::UPDATE,
            "Updates",
            "version update",
        ),
        (
            SettingsTab::About,
            icons::INFO,
            "About",
            "version licences license",
        ),
    ]
}

fn category_matches(label: &str, keywords: &str, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    query
        .split_whitespace()
        .all(|word| label.to_lowercase().contains(word) || keywords.contains(word))
}

/// The row supplies the visible label; keep the checkbox named for assistive technology.
fn settings_checkbox(ui: &mut egui::Ui, checked: &mut bool, label: &str) -> egui::Response {
    kit::toggle(ui, checked, label)
}

impl RecordeApp {
    pub(super) fn render_shortcut_settings(&mut self, ui: &mut egui::Ui) {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            if ui
                .add(kit::secondary_button("Reset all shortcuts to defaults"))
                .on_hover_text("Restore all keyboard shortcuts to their defaults.")
                .clicked()
            {
                self.config.shortcuts = Default::default();
                self.shortcut_capture = None;
                self.shortcut_error = None;
                self.save_config();
            }
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| kit::page_header(ui, "Keyboard", ""),
            );
        });
        kit::hint(
            ui,
            "Choose Change, then press a key with Ctrl (Command on macOS). \
             Escape cancels recording.",
        );
        ui.add_space(kit::space::MD);

        if let Some(command) = self.shortcut_capture {
            let events = ui.input(|i| i.events.clone());
            for event in events {
                if let egui::Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    modifiers,
                    ..
                } = event
                {
                    if key == Key::Escape {
                        self.shortcut_capture = None;
                    } else if let Some(binding) =
                        crate::commands::Shortcut::from_event(key, modifiers)
                    {
                        match self.config.shortcuts.set(command, Some(binding)) {
                            Ok(()) => {
                                self.shortcut_capture = None;
                                self.shortcut_error = None;
                                self.save_config();
                            }
                            Err(error) => self.shortcut_error = Some(error),
                        }
                    }
                }
            }
        }
        let capture_id = ui.id().with("shortcut_capture");
        motion::reveal(ui, capture_id, self.shortcut_capture.is_some(), |ui| {
            let action = self
                .shortcut_capture
                .map(|command| command.label())
                .unwrap_or("");
            kit::callout(
                ui,
                kit::Status::Info,
                &format!("Recording shortcut for {action}. Press a key or Escape to cancel."),
            );
            ui.add_space(kit::space::SM);
        });
        let error_id = ui.id().with("shortcut_error");
        motion::reveal(ui, error_id, self.shortcut_error.is_some(), |ui| {
            kit::callout(
                ui,
                kit::Status::Error,
                self.shortcut_error.as_deref().unwrap_or(""),
            );
            ui.add_space(kit::space::SM);
        });
        kit::section(
            ui,
            "Shortcuts",
            "Shortcuts pause while editing text or using a dialog.",
            |ui| {
                egui::Grid::new("shortcut_settings_grid")
                    .num_columns(3)
                    .spacing(egui::vec2(kit::space::MD, kit::space::SM))
                    .striped(true)
                    .show(ui, |ui| {
                        ui.strong("Action");
                        ui.strong("Shortcut");
                        // The buttons name themselves; no third header.
                        ui.label("");
                        ui.end_row();
                        for &command in crate::commands::Command::ALL {
                            ui.label(command.label())
                                .on_hover_text(command.description());
                            let binding = self
                                .config
                                .shortcuts
                                .get(command)
                                .map(|binding| binding.label());
                            let assigned = binding.is_some();
                            if self.shortcut_capture == Some(command) {
                                kit::status_chip(ui, kit::Status::Info, "Recording");
                            } else if let Some(binding) = binding {
                                ui.label(binding);
                            } else {
                                // Unassigned rows recede so assigned shortcuts scan.
                                ui.label(
                                    RichText::new("Not set")
                                        .size(theme::font_size::SMALL)
                                        .color(theme::text_muted()),
                                );
                            }
                            ui.push_id(command.id(), |ui| {
                                ui.horizontal(|ui| {
                                    if ui
                                        .add(kit::small_button("Change"))
                                        .on_hover_text(
                                            "Record a new keyboard shortcut for this action.",
                                        )
                                        .clicked()
                                    {
                                        self.shortcut_capture = Some(command);
                                        self.shortcut_error = None;
                                    }
                                    // Clear only appears when there is something to clear.
                                    if assigned
                                        && ui
                                            .add(kit::small_button("Clear"))
                                            .on_hover_text(
                                                "Remove the keyboard shortcut for this action.",
                                            )
                                            .clicked()
                                    {
                                        let _ = self.config.shortcuts.set(command, None);
                                        self.shortcut_capture = None;
                                        self.save_config();
                                    }
                                });
                            });
                            ui.end_row();
                        }
                    });
            },
        );
    }

    pub(super) fn render_settings(&mut self, ctx: &Context) {
        if let Some(pending) = &self.diagnostics_pending {
            match pending.try_recv() {
                Ok(ssh) => {
                    self.ssh_diagnostics = Some(ssh);
                    self.diagnostics_pending = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.diagnostics_pending = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100))
                }
            }
        }
        let max_height = (ctx.content_rect().height() - 48.0).max(0.0);
        // A fixed width keeps the side navigation still when a page (such as
        // Add account) has wider content; only the height is resizable.
        let width = settings_width(ctx.content_rect().width());
        egui::Window::new("Settings")
            .collapsible(false)
            .resizable([false, true])
            .default_size([width, 640.0])
            .min_width(width)
            .max_width(width)
            .min_height(420.0)
            .max_height(max_height)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(egui::Frame::window(&ctx.global_style()).inner_margin(egui::Margin::same(20)))
            .show(ctx, |ui| {
                let search_id = ui.id().with("settings_search");
                let mut search =
                    ui.data_mut(|data| data.get_temp::<String>(search_id).unwrap_or_default());

                // Resize supplies the current inner height after the title and frame.
                // Reserve the footer's actual height first: a fixed estimate can make
                // content overflow, which Resize remembers and adds on the next frame.
                let inner_height = ui.available_height().min(max_height).max(0.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), inner_height),
                    egui::Layout::bottom_up(egui::Align::Min),
                    |ui| {
                        ui.horizontal(|ui| {
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.add(kit::secondary_button("Close")).clicked() {
                                        self.settings_open = false;
                                    }
                                    kit::hint(
                                        ui,
                                        "Changes are saved as you make them unless \
                                         a Save button is shown.",
                                    );
                                },
                            );
                        });
                        ui.add_space(kit::space::SM);
                        let content_height = ui.available_height().max(0.0);
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), content_height),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.horizontal_top(|ui| {
                                    ui.allocate_ui_with_layout(
                                        egui::vec2(200.0, content_height),
                                        egui::Layout::top_down(egui::Align::Min),
                                        |ui| {
                                            ui.set_width(200.0);
                                            let changed = ui
                                                .add(
                                                    crate::ui::kit::text_field(&mut search)
                                                        .prefix(
                                                            icons::rich(icons::SEARCH, 18.0)
                                                                .color(theme::text_muted()),
                                                        )
                                                        .margin(egui::Margin::symmetric(10, 7))
                                                        .hint_text("Search settings")
                                                        .desired_width(200.0),
                                                )
                                                .changed();
                                            ui.data_mut(|data| {
                                                data.insert_temp(search_id, search.clone());
                                            });
                                            let categories = settings_categories();
                                            if changed {
                                                if let Some((tab, _, _, _)) = categories
                                                    .iter()
                                                    .find(|(_, _, label, keywords)| {
                                                        category_matches(label, keywords, &search)
                                                    })
                                                {
                                                    self.settings_tab = *tab;
                                                }
                                            }
                                            ui.add_space(kit::space::SM);
                                            egui::ScrollArea::vertical()
                                                .id_salt("settings_navigation")
                                                .max_height(ui.available_height().max(0.0))
                                                .min_scrolled_height(0.0)
                                                .show(ui, |ui| {
                                                    for (tab, icon, label, keywords) in categories {
                                                        if category_matches(
                                                            label, keywords, &search,
                                                        ) && kit::nav_item(
                                                            ui,
                                                            icon,
                                                            label,
                                                            self.settings_tab == tab,
                                                        )
                                                        .clicked()
                                                        {
                                                            self.settings_tab = tab;
                                                        }
                                                    }
                                                });
                                        },
                                    );
                                    ui.add_space(kit::space::MD);
                                    let tab = self.settings_tab;
                                    let fade_id = ui.id().with("settings_page_fade");
                                    for (category, _, _, _) in settings_categories() {
                                        if category != tab {
                                            motion::progress(
                                                ctx,
                                                fade_id.with(category as u8),
                                                false,
                                                motion::FAST,
                                            );
                                        }
                                    }
                                    // ScrollArea inherits its parent's layout. Keep this
                                    // explicit top-down column inside the horizontal row.
                                    let page_width = ui.available_width();
                                    ui.allocate_ui_with_layout(
                                        egui::vec2(page_width, content_height),
                                        egui::Layout::top_down(egui::Align::Min),
                                        |ui| {
                                            ui.set_width(page_width);
                                            egui::ScrollArea::vertical()
                                                .id_salt(("settings_content", tab as u8))
                                                .max_height(content_height)
                                                .min_scrolled_height(0.0)
                                                .auto_shrink([false, false])
                                                .show(ui, |ui| {
                                                    ui.set_width(ui.available_width());
                                                    motion::fade(ui, fade_id.with(tab as u8), true);
                                                    match tab {
                                                        SettingsTab::App => {
                                                            self.render_general_settings(ui)
                                                        }
                                                        SettingsTab::Git => {
                                                            self.render_git_settings(ui)
                                                        }
                                                        SettingsTab::Tools => {
                                                            self.render_tool_settings(ui)
                                                        }
                                                        SettingsTab::Ui => {
                                                            self.render_ui_settings(ctx, ui)
                                                        }
                                                        SettingsTab::Shortcuts => {
                                                            self.render_shortcut_settings(ui)
                                                        }
                                                        SettingsTab::Accounts => {
                                                            self.render_accounts_tab(ui)
                                                        }
                                                        SettingsTab::Repository => {
                                                            self.render_repo_settings(ui)
                                                        }
                                                        SettingsTab::About => self.render_about(ui),
                                                        SettingsTab::Updates => {
                                                            self.render_update_settings(ui)
                                                        }
                                                    }
                                                });
                                        },
                                    );
                                });
                            },
                        );
                    },
                );
                if ui.input(|i| i.key_pressed(Key::Escape)) {
                    self.settings_open = false;
                }
            });
    }

    fn render_general_settings(&mut self, ui: &mut egui::Ui) {
        kit::page_header(ui, "General", "Choose how Recorde works for you.");
        kit::section(
            ui,
            "Author pictures",
            "Choose whether to load pictures for commit authors.",
            |ui| {
                kit::setting_row(
                    ui,
                    "Author pictures",
                    "When enabled, a hash of each author's email is sent to Gravatar. \
                     Pictures are cached for a week.",
                    |ui| {
                        if ui
                            .checkbox(
                                &mut self.config.gravatar,
                                "Show author pictures from Gravatar and GitHub",
                            )
                            .changed()
                        {
                            crate::ui::avatars::set_enabled(self.config.gravatar);
                            self.save_config();
                        }
                    },
                );
            },
        );
        kit::section(
            ui,
            "More settings",
            "Configure Git and the tools you use with Recorde.",
            |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui.add(kit::secondary_button("Git")).clicked() {
                        self.settings_tab = SettingsTab::Git;
                    }
                    if ui.add(kit::secondary_button("Tools")).clicked() {
                        self.settings_tab = SettingsTab::Tools;
                    }
                });
            },
        );
    }

    pub(super) fn update_restart_idle(&self) -> bool {
        let state = self.state.lock().unwrap();
        !self.background_work_pending(&state)
            && !state.bisect_in_progress
            && self.account_task.is_none()
            && self.ssh_task.is_none()
            && !self.repository_browser.is_busy()
            && !state.capabilities.as_ref().is_some_and(|c| {
                c.rebase_in_progress
                    || c.merge_in_progress
                    || c.cherry_pick_in_progress
                    || c.revert_in_progress
            })
    }

    pub(super) fn prepare_update_restart(&mut self) -> Result<(), String> {
        if !self.update_restart_idle() {
            return Err(
                "Finish repository operations, clones and pending account changes \
                 before restarting."
                    .into(),
            );
        }
        if self.repo_ui.conflict_editor.is_some()
            || self.repo_ui.provider_editor.is_some()
            || self.repo_ui.rebase_modal.is_some()
            || unsaved_file_edit(&self.repo_ui, self.repo_sessions.values()).is_some()
            || self.terminal.is_running()
            || self.repo_sessions.values().any(|session| {
                session.conflict_editor.is_some()
                    || session.provider_editor.is_some()
                    || session.rebase_modal.is_some()
            })
        {
            return Err(
                "Save or close file editors in every repository tab, close conflict and review \
                 editors and rebase plans, and stop terminal commands before restarting."
                    .into(),
            );
        }
        let mut paths = self.config.open_repos.clone();
        if let Some(path) = self.state.lock().unwrap().repo_path.clone() {
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
        for path in paths {
            let repository = git2::Repository::open(&path).map_err(|_| {
                format!(
                    "Cannot verify repository state before restart: {}",
                    path.display()
                )
            })?;
            if repository.state() != git2::RepositoryState::Clean
                || repository.path().join("BISECT_LOG").exists()
            {
                return Err(format!(
                    "Finish the Git sequence in {} before restarting.",
                    path.display()
                ));
            }
        }
        self.staging_panel.flush_drafts()?;
        self.persist_navigation();
        self.config.try_save()?;
        Ok(())
    }

    pub(super) fn render_update_settings(&mut self, ui: &mut egui::Ui) {
        kit::page_header(ui, "Updates", "Check for and install Recorde updates.");
        kit::section(ui, "Application updates", "", |ui| {
            let idle = self.update_restart_idle();
            if self.updater.show(ui, &self.config.accounts, idle) {
                match self.prepare_update_restart() {
                    Ok(()) => self.updater.start_apply(),
                    Err(error) => self.updater.report_error(error),
                }
            }
        });
    }

    pub(super) fn render_ui_settings(&mut self, ctx: &Context, ui: &mut egui::Ui) {
        kit::page_header(ui, "Appearance", "Adjust colours, size and animation.");
        kit::section(ui, "Display", "", |ui| {
            kit::setting_row(
                ui,
                "Colour palette",
                "Choose the colours used throughout Recorde.",
                |ui| {
                    let previous = self.config.theme_palette;
                    let response = egui::ComboBox::from_id_salt("settings_colour_palette")
                        .selected_text(previous.label())
                        // Tall enough to show every theme and group heading
                        // without scrolling; egui's default fits about four.
                        .height(560.0)
                        .show_ui(ui, |ui| {
                            // Grouped by mode: Classic and Siege21 follow the Dark
                            // theme setting; the others are always light or dark.
                            for (heading, mode) in [
                                ("Light and dark", None),
                                ("Dark", Some(true)),
                                ("Light", Some(false)),
                            ] {
                                if mode.is_some() {
                                    ui.separator();
                                }
                                ui.label(
                                    RichText::new(heading)
                                        .size(theme::font_size::SMALL)
                                        .color(theme::text_muted()),
                                );
                                for palette in theme::Palette::ALL {
                                    if palette.fixed_dark() == mode {
                                        ui.selectable_value(
                                            &mut self.config.theme_palette,
                                            palette,
                                            palette.label(),
                                        );
                                    }
                                }
                            }
                        })
                        .response;
                    response.widget_info(|| {
                        let mut info = egui::WidgetInfo::labeled(
                            egui::WidgetType::ComboBox,
                            ui.is_enabled(),
                            "Colour palette",
                        );
                        info.current_text_value = Some(self.config.theme_palette.label().into());
                        info
                    });
                    if previous != self.config.theme_palette {
                        self.save_config();
                        theme::apply_palette(
                            ctx,
                            self.dark_theme,
                            self.high_contrast,
                            self.config.theme_palette,
                        );
                    }
                },
            );
            kit::row_divider(ui);
            let fixed_mode = self.config.theme_palette.fixed_dark();
            let dark_help = match fixed_mode {
                Some(dark) => format!(
                    "{} is always {}. Classic and Siege21 can switch.",
                    self.config.theme_palette.label(),
                    if dark { "dark" } else { "light" }
                ),
                None => "Use darker surfaces throughout the app.".to_owned(),
            };
            kit::setting_row(ui, "Dark theme", &dark_help, |ui| {
                if let Some(dark) = fixed_mode {
                    // Show the theme's own mode; the preference is kept for
                    // when a light-and-dark palette is chosen again.
                    let mut shown = dark;
                    ui.add_enabled_ui(false, |ui| settings_checkbox(ui, &mut shown, "Dark theme"));
                    return;
                }
                {
                    if settings_checkbox(ui, &mut self.dark_theme, "Dark theme").changed() {
                        self.config.dark_theme = self.dark_theme;
                        self.save_config();
                        theme::apply_palette(
                            ctx,
                            self.dark_theme,
                            self.high_contrast,
                            self.config.theme_palette,
                        );
                    }
                }
            });
            kit::row_divider(ui);
            kit::setting_row(
                ui,
                "High contrast",
                "Increase contrast between interface elements.",
                |ui| {
                    if settings_checkbox(ui, &mut self.high_contrast, "High contrast").changed() {
                        self.config.high_contrast = self.high_contrast;
                        self.save_config();
                        theme::apply_palette(
                            ctx,
                            self.dark_theme,
                            self.high_contrast,
                            self.config.theme_palette,
                        );
                    }
                },
            );
            kit::row_divider(ui);
            kit::setting_row(
                ui,
                "Colour-blind-safe graph colours",
                "Use a more distinguishable graph palette.",
                |ui| {
                    if settings_checkbox(
                        ui,
                        &mut self.config.colorblind_graph,
                        "Colour-blind-safe graph colours",
                    )
                    .changed()
                    {
                        self.commit_graph_view
                            .set_colorblind(self.config.colorblind_graph);
                        self.sidebar.set_colorblind(self.config.colorblind_graph);
                        self.save_config();
                    }
                },
            );
            kit::row_divider(ui);
            kit::setting_row(
                ui,
                "UI scale",
                "1.0 is your display's native scale. Click Apply to use the new size.",
                |ui| {
                    ui.horizontal_wrapped(|ui| {
                        let response = kit::slider(
                            ui,
                            egui::Slider::new(&mut self.config.font_scale, 0.75..=2.0)
                                .step_by(0.25),
                        );
                        response.widget_info(|| {
                            egui::WidgetInfo::slider(
                                ui.is_enabled(),
                                self.config.font_scale as f64,
                                "UI scale",
                            )
                        });
                        if ui
                            .add(kit::secondary_button("Apply"))
                            .on_hover_text("Apply and save the selected interface size.")
                            .clicked()
                        {
                            theme::apply_ui_scale(ctx, self.config.font_scale);
                            self.save_config();
                        }
                    });
                },
            );
            kit::row_divider(ui);
            kit::setting_row(
                ui,
                "Motion",
                "Animations for panels, lists and menus. \"Match system setting\" follows \
                 Windows' \"Animation effects\" option.",
                |ui| {
                    let previous = self.config.motion;
                    egui::ComboBox::from_id_salt("motion_preference")
                        .selected_text(previous.label())
                        .show_ui(ui, |ui| {
                            for preference in motion::MotionPreference::ALL {
                                ui.selectable_value(
                                    &mut self.config.motion,
                                    preference,
                                    preference.label(),
                                );
                            }
                        });
                    if previous != self.config.motion {
                        motion::apply(ctx, self.config.motion);
                        theme::apply_palette(
                            ctx,
                            self.dark_theme,
                            self.high_contrast,
                            self.config.theme_palette,
                        );
                        self.save_config();
                    }
                },
            );
        });
    }

    fn render_git_readiness(&mut self, ui: &mut egui::Ui) {
        git_readiness_status(ui, &mut self.git_readiness);
    }

    fn render_git_settings(&mut self, ui: &mut egui::Ui) {
        kit::page_header(
            ui,
            "Git",
            "Choose how Recorde retrieves and combines remote changes.",
        );
        if let Some(error) = &self.config.load_error {
            kit::callout(ui, kit::Status::Error, error);
            ui.add_space(kit::space::MD);
        }
        kit::section(
            ui,
            "Installed Git",
            "System Git runs repository commands. Git LFS is an optional separate installation.",
            |ui| {
                self.render_git_readiness(ui);
                ui.add_space(kit::space::SM);
                ui.label("Git executable (absolute path; leave empty to use PATH)");
                ui.text_edit_singleline(&mut self.git_executable_input);
                if ui
                    .add(kit::secondary_button("Apply Git executable"))
                    .clicked()
                {
                    self.config.git_executable = self.git_executable_input.trim().to_owned();
                    crate::git::readiness::configure(&self.config.git_executable);
                    self.git_readiness.retry();
                    self.save_config();
                }
            },
        );
        kit::section(
            ui,
            "Pull",
            "Pull brings remote changes into your current branch.",
            |ui| {
                kit::setting_row(
                    ui,
                    "Default pull strategy",
                    "Choose how to combine incoming commits with your work.",
                    |ui| {
                        for (strategy, label, help) in [
                            (
                                PullStrategy::GitConfig,
                                "Use Git configuration (recommended)",
                                "Follow your existing Git settings.",
                            ),
                            (
                                PullStrategy::Merge,
                                "Merge",
                                "Join both histories with a merge commit when needed.",
                            ),
                            (
                                PullStrategy::Rebase,
                                "Rebase",
                                "Replay your commits on top of incoming changes.",
                            ),
                            (
                                PullStrategy::FfOnly,
                                "Fast-forward only",
                                "Update only when histories have not diverged.",
                            ),
                        ] {
                            if crate::ui::kit::radio_value(
                                ui,
                                &mut self.config.pull_strategy,
                                strategy,
                                label,
                            )
                            .on_hover_text(help)
                            .changed()
                            {
                                self.save_config();
                            }
                            kit::hint(ui, help);
                        }
                    },
                );
            },
        );
        kit::section(
            ui,
            "Fetch",
            "Fetch checks the remote for new commits without changing your files.",
            |ui| {
                kit::setting_row(
                    ui,
                    "Prune deleted remote branches on fetch",
                    "Remove local tracking references to branches deleted on the remote.",
                    |ui| {
                        if settings_checkbox(
                            ui,
                            &mut self.config.fetch_prune,
                            "Prune deleted remote branches on fetch",
                        )
                        .changed()
                        {
                            self.save_config();
                        }
                    },
                );
                kit::row_divider(ui);
                kit::setting_row(
                    ui,
                    "Auto-fetch on repo open",
                    "Automatically check for remote changes.",
                    |ui| {
                        if settings_checkbox(
                            ui,
                            &mut self.config.auto_fetch,
                            "Auto-fetch on repo open",
                        )
                        .changed()
                        {
                            self.save_config();
                        }
                    },
                );
                kit::row_divider(ui);
                kit::setting_row(
                    ui,
                    "Interval (seconds)",
                    "Available when auto-fetch is on.",
                    |ui| {
                        ui.add_enabled_ui(self.config.auto_fetch, |ui| {
                            let mut value = self.config.auto_fetch_interval_secs.to_string();
                            if ui
                                .add(
                                    crate::ui::kit::text_field(&mut value)
                                        .horizontal_align(egui::Align::Max)
                                        .suffix(
                                            RichText::new("seconds")
                                                .size(theme::font_size::SMALL)
                                                .color(theme::text_muted()),
                                        )
                                        .desired_width(140.0),
                                )
                                .on_disabled_hover_text(
                                    "Turn on auto-fetch to change its interval.",
                                )
                                .changed()
                            {
                                if let Ok(value) = value.trim().parse::<u32>() {
                                    self.config.auto_fetch_interval_secs = value;
                                    self.save_config();
                                }
                            }
                        });
                    },
                );
            },
        );
    }

    fn render_tool_settings(&mut self, ui: &mut egui::Ui) {
        kit::page_header(
            ui,
            "Tools",
            "Choose external applications for editing and comparing files.",
        );
        kit::section(
            ui,
            "External tools",
            "Enter a command and save each change.",
            |ui| {
                kit::setting_row(
                    ui,
                    "Default editor",
                    "Used by Repository > Open in Editor. Arguments are supported, \
                     for example code --reuse-window.",
                    |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.add(
                                crate::ui::kit::text_field(&mut self.default_editor_input)
                                    .hint_text("code, codium, zed, vim, ...")
                                    .desired_width(200.0),
                            );
                            if ui
                                .add_enabled(
                                    self.default_editor_input.trim() != self.config.default_editor,
                                    kit::small_button("Save"),
                                )
                                .on_hover_text("Save the editor command for future launches.")
                                .on_disabled_hover_text("No unsaved changes to the editor command.")
                                .clicked()
                            {
                                self.config.default_editor =
                                    self.default_editor_input.trim().to_string();
                                self.save_config();
                            }
                            if ui
                                .add(kit::small_button("Test in editor"))
                                .on_hover_text(
                                    "Open the current repository using the saved command.",
                                )
                                .clicked()
                            {
                                if let Err(e) = open_repo_in_editor(
                                    self.state.lock().unwrap().repo_path.clone(),
                                    &self.config.default_editor,
                                ) {
                                    self.state.lock().unwrap().error = Some(e);
                                }
                            }
                        });
                    },
                );
                kit::row_divider(ui);
                kit::setting_row(
                    ui,
                    "Terminal",
                    "For example: xterm, gnome-terminal, konsole.",
                    |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.add(
                                crate::ui::kit::text_field(&mut self.terminal_input)
                                    .hint_text("xterm, gnome-terminal, konsole, ...")
                                    .desired_width(200.0),
                            );
                            if ui
                                .add_enabled(
                                    self.terminal_input.trim() != self.config.terminal,
                                    kit::small_button("Save"),
                                )
                                .on_hover_text("Save the terminal command for future launches.")
                                .on_disabled_hover_text(
                                    "No unsaved changes to the terminal command.",
                                )
                                .clicked()
                            {
                                self.config.terminal = self.terminal_input.trim().to_string();
                                self.save_config();
                            }
                        });
                    },
                );
                kit::row_divider(ui);
                kit::setting_row(
                    ui,
                    "Diff tool",
                    "For example: meld, kdiff3, vimdiff.",
                    |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.add(
                                crate::ui::kit::text_field(&mut self.diff_tool_input)
                                    .hint_text("meld, kdiff3, vimdiff, ...")
                                    .desired_width(200.0),
                            );
                            if ui
                                .add_enabled(
                                    self.diff_tool_input.trim() != self.config.diff_tool,
                                    kit::small_button("Save"),
                                )
                                .on_hover_text("Save the command used to compare files.")
                                .on_disabled_hover_text(
                                    "No unsaved changes to the diff tool command.",
                                )
                                .clicked()
                            {
                                self.config.diff_tool = self.diff_tool_input.trim().to_string();
                                self.save_config();
                            }
                        });
                    },
                );
                kit::row_divider(ui);
                kit::setting_row(ui, "Merge tool", "For example: meld or kdiff3.", |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.add(
                            crate::ui::kit::text_field(&mut self.merge_tool_input)
                                .hint_text("meld, kdiff3, ...")
                                .desired_width(200.0),
                        );
                        if ui
                            .add_enabled(
                                self.merge_tool_input.trim() != self.config.merge_tool,
                                kit::small_button("Save"),
                            )
                            .on_hover_text("Save the command used to resolve merge conflicts.")
                            .on_disabled_hover_text("No unsaved changes to the merge tool command.")
                            .clicked()
                        {
                            self.config.merge_tool = self.merge_tool_input.trim().to_string();
                            self.save_config();
                        }
                    });
                });
            },
        );
    }

    pub(super) fn spawn_repo_settings_operation(
        &self,
        path: std::path::PathBuf,
        work: impl FnOnce(&std::path::Path) -> Result<(), String> + Send + 'static,
    ) {
        let generation = self.repo_settings.generation.clone();
        crate::git::mutation::spawn_operation(path, self.state.clone(), move |repo| {
            let result = work(repo);
            // Reload on failure too: a multi-step save may have partially succeeded.
            generation.fetch_add(1, std::sync::atomic::Ordering::Release);
            result
        });
    }

    pub(super) fn render_repo_settings(&mut self, ui: &mut egui::Ui) {
        let (
            repo_path,
            repo_epoch,
            head_branch,
            ahead_behind,
            dirty_count,
            conflict_count,
            caps,
            submodules,
            worktrees,
        ) = {
            let s = self.state.lock().unwrap();
            (
                s.repo_path.clone(),
                s.repo_epoch,
                s.head_branch.clone().unwrap_or_else(|| "—".into()),
                s.ahead_behind,
                s.dirty_count,
                s.conflicted_files.len(),
                s.capabilities.clone(),
                s.submodules.clone(),
                s.worktrees.clone(),
            )
        };
        let Some(path) = repo_path else {
            self.repo_settings.visible = false;
            kit::page_header(ui, "Repository", "Change settings for the open repository.");
            kit::empty_state(
                ui,
                icons::FOLDER,
                "No repository open",
                "Open a repository to change its settings.",
                None,
            );
            return;
        };

        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| kit::display_path(&path));
        kit::page_header(
            ui,
            &format!("Settings for {name}"),
            &kit::display_path(&path),
        );
        self.repo_settings
            .poll(&path, repo_epoch, &head_branch, ui.ctx());
        let snapshot = match self.repo_settings.snapshot.clone() {
            Some(Ok(snapshot)) => snapshot,
            Some(Err(error)) => {
                kit::section(ui, "Repository settings", "", |ui| {
                    kit::callout(ui, kit::Status::Error, &error);
                    ui.add_space(kit::space::SM);
                    if ui
                        .add(kit::secondary_button("Reload repository settings"))
                        .on_hover_text("Try loading this repository's settings again.")
                        .clicked()
                    {
                        self.repo_settings.visible = false;
                    }
                });
                return;
            }
            None => {
                kit::section(ui, "Repository settings", "", |ui| {
                    kit::status_chip(ui, kit::Status::Info, "Loading…");
                });
                return;
            }
        };
        kit::section(
            ui,
            "Recovery snapshots",
            "Restore saved files and repository state after an operation.",
            |ui| {
                kit::disclosure(
                    ui,
                    "recovery_details",
                    "Manage recovery snapshots",
                    false,
                    |ui| {
                        kit::disclosure(
                            ui,
                            "snapshot_details",
                            "How snapshots work",
                            false,
                            |ui| {
                                kit::hint(
                                    ui,
                                    "Snapshots preserve the index and non-ignored files before \
                                     commit, reset and discard, including before the first commit. \
                                     Undo checks for later edits and saves a snapshot for redo. \
                                     Ignored files are excluded.",
                                );
                            },
                        );
                        kit::row_divider(ui);
                        kit::setting_row(
                            ui,
                            "Automatic cleanup",
                            "Cleanup always keeps the latest 20 snapshots, pinned snapshots \
                             and incomplete operations.",
                            |ui| {
                                let mut retention = snapshot.retention_days;
                                let old_retention = retention;
                                egui::ComboBox::from_label("Automatic cleanup")
                                    .selected_text(if retention <= 0 {
                                        "Keep all snapshots".into()
                                    } else {
                                        format!("After {retention} days")
                                    })
                                    .show_ui(ui, |ui| {
                                        for (days, label) in [
                                            (0, "Keep all snapshots"),
                                            (30, "After 30 days"),
                                            (90, "After 90 days"),
                                        ] {
                                            ui.selectable_value(&mut retention, days, label);
                                        }
                                    });
                                if retention != old_retention {
                                    self.spawn_repo_settings_operation(path.clone(), move |repo| {
                                        git2::Repository::open(repo)
                                            .and_then(|repo| repo.config())
                                            .and_then(|mut config| {
                                                config.set_i64(
                                                    "recorde.recoveryRetentionDays",
                                                    retention,
                                                )
                                            })
                                            .map_err(|error| error.to_string())?;
                                        crate::git::recovery::prune(repo)
                                    });
                                }
                            },
                        );
                        kit::row_divider(ui);
                        let saved = &snapshot.recovery;
                        if saved.is_empty() {
                            kit::hint(ui, "No recovery snapshots yet.");
                        }
                        egui::ScrollArea::vertical()
                            .id_salt("recovery_snapshots")
                            .max_height(260.0)
                            .show(ui, |ui| {
                                for (index, checkpoint) in saved.iter().enumerate() {
                                    if index > 0 {
                                        kit::row_divider(ui);
                                    }
                                    ui.push_id(&checkpoint.id, |ui| {
                                        kit::setting_row(
                                            ui,
                                            &checkpoint.label,
                                            &checkpoint.id,
                                            |ui| {
                                                ui.horizontal_wrapped(|ui| {
                                                    let mut pinned = checkpoint.pinned;
                                                    if ui
                                                        .checkbox(&mut pinned, "Keep")
                                                        .on_hover_text(
                                                            "Keep this snapshot during \
                                                             automatic cleanup.",
                                                        )
                                                        .changed()
                                                    {
                                                        let id = checkpoint.id.clone();
                                                        self.spawn_repo_settings_operation(
                                                            path.clone(),
                                                            move |repo| {
                                                                crate::git::recovery::set_pinned(
                                                                    repo, &id, pinned,
                                                                )
                                                            },
                                                        );
                                                    }
                                                    if ui
                                                        .add(kit::small_button("Restore..."))
                                                        .on_hover_text(
                                                            "Review restoring the saved files \
                                                             and repository state.",
                                                        )
                                                        .clicked()
                                                    {
                                                        self.repo_ui.pending_recovery_id =
                                                            Some(checkpoint.id.clone());
                                                    }
                                                    if ui
                                                        .add(kit::danger_outline_button(
                                                            "Delete...",
                                                        ))
                                                        .on_hover_text(
                                                            "Review deleting this \
                                                             recovery snapshot.",
                                                        )
                                                        .clicked()
                                                    {
                                                        self.repo_ui.pending_recovery_id = Some(
                                                            format!("delete:{}", checkpoint.id),
                                                        );
                                                    }
                                                });
                                            },
                                        );
                                        let restoring = self.repo_ui.pending_recovery_id.as_ref()
                                            == Some(&checkpoint.id);
                                        let restore_id = ui.id().with("restore_snapshot");
                                        motion::reveal(ui, restore_id, restoring, |ui| {
                                            ui.add_enabled_ui(restoring, |ui| {
                                                ui.add_space(kit::space::SM);
                                                kit::callout(
                                                    ui,
                                                    kit::Status::Warning,
                                                    "Restore the saved HEAD, index and file \
                                                     contents? Later changes must be preserved \
                                                     first; the saved snapshot remains available.",
                                                );
                                                ui.add_space(kit::space::SM);
                                                ui.horizontal_wrapped(|ui| {
                                                    if ui
                                                        .add(kit::primary_button(
                                                            "Restore snapshot",
                                                        ))
                                                        .clicked()
                                                    {
                                                        let checkpoint = checkpoint.clone();
                                                        self.repo_ui.pending_recovery_id = None;
                                                        self.spawn_repo_settings_operation(
                                                            path.clone(),
                                                            move |repo| {
                                                                crate::git::recovery::restore(
                                                                    repo,
                                                                    &checkpoint,
                                                                )
                                                            },
                                                        );
                                                    }
                                                    if ui
                                                        .add(kit::secondary_button("Cancel"))
                                                        .clicked()
                                                    {
                                                        self.repo_ui.pending_recovery_id = None;
                                                    }
                                                });
                                            });
                                        });
                                        let delete_key = format!("delete:{}", checkpoint.id);
                                        let deleting = self.repo_ui.pending_recovery_id.as_deref()
                                            == Some(delete_key.as_str());
                                        let delete_id = ui.id().with("delete_snapshot");
                                        motion::reveal(ui, delete_id, deleting, |ui| {
                                            ui.add_enabled_ui(deleting, |ui| {
                                                ui.add_space(kit::space::SM);
                                                kit::callout(
                                                    ui,
                                                    kit::Status::Warning,
                                                    "Delete this recovery snapshot and its \
                                                     protected ref? Its saved data may be removed \
                                                     by Git garbage collection.",
                                                );
                                                ui.add_space(kit::space::SM);
                                                ui.horizontal_wrapped(|ui| {
                                                    if ui
                                                        .add(kit::danger_button("Delete snapshot"))
                                                        .clicked()
                                                    {
                                                        let checkpoint = checkpoint.clone();
                                                        self.repo_ui.pending_recovery_id = None;
                                                        self.spawn_repo_settings_operation(
                                                            path.clone(),
                                                            move |repo| {
                                                                crate::git::recovery::remove(
                                                                    repo,
                                                                    &checkpoint,
                                                                )
                                                            },
                                                        );
                                                    }
                                                    if ui
                                                        .add(kit::secondary_button("Keep snapshot"))
                                                        .clicked()
                                                    {
                                                        self.repo_ui.pending_recovery_id = None;
                                                    }
                                                });
                                            });
                                        });
                                    });
                                }
                            });
                    },
                );
            },
        );
        kit::section(
            ui,
            "Maintenance",
            "Advanced repository operations can change the files checked out on disk.",
            |ui| {
                kit::disclosure(
                    ui,
                    "checkout_history",
                    "Checkout and history",
                    false,
                    |ui| {
                        let caps = self
                            .state
                            .lock()
                            .unwrap()
                            .capabilities
                            .clone()
                            .unwrap_or_default();
                        kit::setting_row(
                            ui,
                            "History",
                            "Build Git's history index for faster graph loading. \
                         Large repositories may take a few minutes.",
                            |ui| {
                                kit::status_chip(
                                    ui,
                                    kit::Status::Neutral,
                                    if caps.is_shallow {
                                        "Shallow history"
                                    } else {
                                        "Full history"
                                    },
                                );
                                ui.add_space(kit::space::SM);
                                if ui
                                .add_enabled(
                                    !caps.is_shallow,
                                    kit::secondary_button("Optimize history loading"),
                                )
                                .on_hover_text(
                                    "Build a local history index to speed up loading the graph.",
                                )
                                .on_disabled_hover_text("Fetch full history before optimizing it.")
                                .clicked()
                            {
                                self.state.lock().unwrap().remote_op =
                                    Some("Optimize history loading".into());
                                self.spawn_repo_settings_operation(
                                    path.clone(),
                                    crate::git::lifecycle::optimize_history,
                                );
                            }
                                let fetch_id = ui.id().with("fetch_full_history");
                                motion::reveal(ui, fetch_id, caps.is_shallow, |ui| {
                                    if ui
                                        .add_enabled(
                                            caps.is_shallow,
                                            kit::secondary_button("Fetch full history"),
                                        )
                                        .on_hover_text(
                                            "Download the missing older commits. Large histories \
                                         may take time and use additional disk space.",
                                        )
                                        .clicked()
                                    {
                                        self.spawn_repo_settings_operation(path.clone(), |repo| {
                                            crate::git::lifecycle::fetch_full_history(repo)
                                        });
                                    }
                                });
                            },
                        );
                        kit::row_divider(ui);
                        kit::setting_row(
                            ui,
                            "Directories to include, one per line (cone mode)",
                            "Sparse checkout keeps only selected directories on disk. Root files \
                         remain included. Applying or disabling sparse checkout requires \
                         a clean worktree and index.",
                            |ui| {
                                kit::status_chip(
                                    ui,
                                    kit::Status::Neutral,
                                    if caps.is_sparse {
                                        "Sparse checkout is enabled"
                                    } else {
                                        "All directories are checked out"
                                    },
                                );
                                ui.add_space(kit::space::SM);
                                ui.add(
                                    egui::TextEdit::multiline(&mut self.repo_ui.sparse_paths)
                                        .desired_width(ui.available_width())
                                        .desired_rows(3),
                                );
                                ui.add_space(kit::space::SM);
                                ui.horizontal_wrapped(|ui| {
                                    if ui
                                    .add_enabled(
                                        !self.repo_ui.sparse_paths.trim().is_empty(),
                                        kit::secondary_button("Apply sparse checkout"),
                                    )
                                    .on_hover_text(
                                        "Keep the listed directories and root files checked out.",
                                    )
                                    .on_disabled_hover_text("Enter at least one directory.")
                                    .clicked()
                                {
                                    let directories = self
                                        .repo_ui
                                        .sparse_paths
                                        .lines()
                                        .map(str::to_string)
                                        .collect();
                                    self.spawn_repo_settings_operation(path.clone(), move |repo| {
                                        crate::git::lifecycle::set_sparse(repo, Some(directories))
                                    });
                                }
                                });
                                let disable_id = ui.id().with("disable_sparse_checkout");
                                motion::reveal(ui, disable_id, caps.is_sparse, |ui| {
                                    if ui
                                        .add_enabled(
                                            caps.is_sparse,
                                            kit::secondary_button("Disable sparse checkout"),
                                        )
                                        .on_hover_text(
                                            "Check out all directories again. Requires no \
                                         uncommitted changes.",
                                        )
                                        .clicked()
                                    {
                                        self.spawn_repo_settings_operation(path.clone(), |repo| {
                                            crate::git::lifecycle::set_sparse(repo, None)
                                        });
                                    }
                                });
                            },
                        );
                    },
                );
                kit::row_divider(ui);
                kit::disclosure(ui, "worktrees", "Worktrees", false, |ui| {
                    kit::setting_row(
                        ui,
                        "New branch",
                        "Create a separate checkout on a new branch so you can work on \
                         two branches at once.",
                        |ui| {
                            ui.text_edit_singleline(&mut self.repo_ui.worktree_branch_input);
                            ui.add_space(kit::space::SM);
                            if ui
                                .add_enabled(
                                    !self.repo_ui.worktree_branch_input.trim().is_empty(),
                                    kit::secondary_button("Create worktree..."),
                                )
                                .on_hover_text("Choose a directory for the new checkout.")
                                .on_disabled_hover_text("Enter a name for the new branch.")
                                .clicked()
                            {
                                if let Some(destination) = rfd::FileDialog::new()
                                    .set_title("Choose the new worktree directory")
                                    .save_file()
                                {
                                    let branch =
                                        self.repo_ui.worktree_branch_input.trim().to_string();
                                    self.spawn_repo_settings_operation(path.clone(), move |repo| {
                                        crate::git::worktrees::add(repo, &destination, &branch)
                                    });
                                }
                            }
                        },
                    );
                    for worktree in &worktrees {
                        kit::row_divider(ui);
                        ui.push_id(&worktree.path, |ui| {
                            kit::setting_row(
                                ui,
                                worktree.branch.as_deref().unwrap_or("Detached"),
                                &worktree.path,
                                |ui| {
                                    let missing = !worktree.is_main && worktree.prunable;
                                    let missing_id = ui.id().with("missing_worktree");
                                    motion::reveal(ui, missing_id, missing, |ui| {
                                        kit::status_chip(
                                            ui,
                                            kit::Status::Warning,
                                            "Missing worktree",
                                        );
                                        ui.add_space(kit::space::XS);
                                    });
                                    ui.horizontal_wrapped(|ui| {
                                        if ui
                                            .add(kit::small_button("Open"))
                                            .on_hover_text(format!(
                                                "Open this checkout in Recorde: {}",
                                                worktree.path
                                            ))
                                            .clicked()
                                        {
                                            load_repo(
                                                worktree.path.clone().into(),
                                                self.state.clone(),
                                            );
                                        }
                                        if !worktree.is_main {
                                            let locked = !worktree.locked;
                                            let label =
                                                if worktree.locked { "Unlock" } else { "Lock" };
                                            if ui
                                                .add(kit::small_button(label))
                                                .on_hover_text(if worktree.locked {
                                                    "Allow Git to remove this checkout."
                                                } else {
                                                    "Protect this checkout from removal by Git."
                                                })
                                                .clicked()
                                            {
                                                let destination =
                                                    std::path::PathBuf::from(&worktree.path);
                                                self.spawn_repo_settings_operation(
                                                    path.clone(),
                                                    move |repo| {
                                                        crate::git::worktrees::set_locked(
                                                            repo,
                                                            &destination,
                                                            locked,
                                                        )
                                                    },
                                                );
                                            }
                                            if ui
                                                .add(kit::danger_outline_button("Remove..."))
                                                .on_hover_text(
                                                    "Review removing this checkout. \
                                                     Its branch will be retained.",
                                                )
                                                .clicked()
                                            {
                                                self.repo_ui.remove_worktree_pending =
                                                    Some(worktree.path.clone());
                                            }
                                        }
                                    });
                                },
                            );
                            let removing = self.repo_ui.remove_worktree_pending.as_ref()
                                == Some(&worktree.path);
                            let remove_id = ui.id().with("remove_worktree");
                            motion::reveal(ui, remove_id, removing, |ui| {
                                ui.add_enabled_ui(removing, |ui| {
                                    ui.add_space(kit::space::SM);
                                    kit::callout(
                                        ui,
                                        kit::Status::Warning,
                                        &format!(
                                            "Remove {}? The branch is retained. Git will refuse \
                                             dirty or locked worktrees.",
                                            worktree.path
                                        ),
                                    );
                                    ui.add_space(kit::space::SM);
                                    ui.horizontal_wrapped(|ui| {
                                        if ui.add(kit::danger_button("Remove worktree")).clicked() {
                                            let destination =
                                                std::path::PathBuf::from(&worktree.path);
                                            self.repo_ui.remove_worktree_pending = None;
                                            self.spawn_repo_settings_operation(
                                                path.clone(),
                                                move |repo| {
                                                    crate::git::worktrees::remove(
                                                        repo,
                                                        &destination,
                                                    )
                                                },
                                            );
                                        }
                                        if ui.add(kit::secondary_button("Cancel")).clicked() {
                                            self.repo_ui.remove_worktree_pending = None;
                                        }
                                    });
                                });
                            });
                        });
                    }
                });
                kit::row_divider(ui);
                kit::disclosure(ui, "submodules", "Submodules", false, |ui| {
                    kit::hint(
                        ui,
                        "Submodules are separate repositories included inside this repository.",
                    );
                    ui.add_space(kit::space::SM);
                    if submodules.is_empty() {
                        kit::hint(ui, "No submodules configured.");
                    }
                    for (index, module) in submodules.iter().enumerate() {
                        if index > 0 {
                            kit::row_divider(ui);
                        }
                        ui.push_id(&module.path, |ui| {
                            kit::setting_row(
                                ui,
                                &module.name,
                                if module.initialized {
                                    "Update the files to the revision recorded by this repository."
                                } else {
                                    "Initialize to download and check out this submodule."
                                },
                                |ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        if ui
                                            .add(kit::small_button(if module.initialized {
                                                "Update"
                                            } else {
                                                "Initialize"
                                            }))
                                            .on_hover_text(
                                                "Check out the submodule revision recorded \
                                                 by this repository.",
                                            )
                                            .clicked()
                                        {
                                            let module = module.path.clone();
                                            self.spawn_repo_settings_operation(
                                                path.clone(),
                                                move |repo| {
                                                    crate::git::submodules::update(repo, &module)
                                                },
                                            );
                                        }
                                        if ui
                                            .add(kit::small_button("Sync URL"))
                                            .on_hover_text(
                                                "Copy the configured submodule URL into \
                                                 your local Git settings.",
                                            )
                                            .clicked()
                                        {
                                            let module = module.path.clone();
                                            self.spawn_repo_settings_operation(
                                                path.clone(),
                                                move |repo| {
                                                    crate::git::submodules::sync(repo, &module)
                                                },
                                            );
                                        }
                                        if module.initialized
                                            && ui
                                                .add(kit::small_button("Open"))
                                                .on_hover_text(
                                                    "Open this submodule as a repository \
                                                     in Recorde.",
                                                )
                                                .clicked()
                                        {
                                            load_repo(path.join(&module.path), self.state.clone());
                                        }
                                    });
                                },
                            );
                        });
                    }
                });
            },
        );
        kit::section(
            ui,
            "Repository health",
            "Current repository status.",
            |ui| {
                kit::setting_row(
                    ui,
                    "Branch",
                    "Your current branch and its relationship to the tracked remote branch.",
                    |ui| {
                        ui.label(&head_branch);
                        let tracking = match ahead_behind {
                            Some((ahead, behind)) => {
                                format!("{ahead} commits ahead, {behind} commits behind")
                            }
                            None => "No upstream branch configured.".into(),
                        };
                        kit::hint(ui, &tracking);
                    },
                );
                kit::row_divider(ui);
                kit::setting_row(
                    ui,
                    "Uncommitted changes",
                    "Files in this checkout that differ from the last commit.",
                    |ui| {
                        if dirty_count == 0 && conflict_count == 0 {
                            kit::status_chip(ui, kit::Status::Ok, "none");
                        } else {
                            ui.horizontal_wrapped(|ui| {
                                if dirty_count > 0 {
                                    kit::status_chip(
                                        ui,
                                        kit::Status::Info,
                                        &format!("{dirty_count} changed"),
                                    );
                                }
                                if conflict_count > 0 {
                                    kit::status_chip(
                                        ui,
                                        kit::Status::Warning,
                                        &format!("{conflict_count} conflicted"),
                                    );
                                }
                            });
                        }
                    },
                );
                if let Some(ref caps) = caps {
                    kit::row_divider(ui);
                    kit::setting_row(
                        ui,
                        "State",
                        "Finish an in-progress Git operation before starting another.",
                        |ui| {
                            let state = if caps.rebase_in_progress {
                                "Rebasing"
                            } else if caps.merge_in_progress {
                                "Merging"
                            } else if caps.cherry_pick_in_progress {
                                "Cherry-picking"
                            } else if caps.revert_in_progress {
                                "Reverting"
                            } else {
                                "Normal"
                            };
                            kit::status_chip(
                                ui,
                                if state == "Normal" {
                                    kit::Status::Ok
                                } else {
                                    kit::Status::Info
                                },
                                state,
                            );
                        },
                    );
                    kit::row_divider(ui);
                    kit::setting_row(
                        ui,
                        "LFS",
                        "Git Large File Storage keeps large file contents outside regular commits.",
                        |ui| {
                            if !self
                                .git_readiness
                                .value
                                .as_ref()
                                .map_or(caps.lfs_installed, |value| value.lfs_version.is_some())
                            {
                                kit::status_chip(ui, kit::Status::Neutral, "not installed");
                            } else if !caps.lfs_active {
                                kit::status_chip(ui, kit::Status::Neutral, "installed, not used");
                            } else {
                                kit::status_chip(ui, kit::Status::Ok, "active");
                                if !caps.lfs_patterns.is_empty() {
                                    kit::hint(ui, &caps.lfs_patterns.join(", "));
                                }
                            }
                        },
                    );
                    kit::row_divider(ui);
                    kit::setting_row(
                        ui,
                        "Submodules",
                        "Separate repositories included inside this repository.",
                        |ui| {
                            if submodules.is_empty() {
                                kit::status_chip(ui, kit::Status::Neutral, "none");
                            } else {
                                let tooltip = submodules
                                    .iter()
                                    .map(|module| {
                                        let status = if module.initialized {
                                            "initialized"
                                        } else {
                                            "not initialized"
                                        };
                                        format!("{} ({status})", module.name)
                                    })
                                    .collect::<Vec<_>>()
                                    .join("\n");
                                kit::status_chip(
                                    ui,
                                    kit::Status::Info,
                                    &submodules.len().to_string(),
                                )
                                .on_hover_text(tooltip);
                            }
                        },
                    );
                    kit::row_divider(ui);
                    kit::setting_row(
                        ui,
                        "Worktrees",
                        "Separate checkouts that share this repository's history.",
                        |ui| {
                            if worktrees.len() <= 1 {
                                kit::status_chip(ui, kit::Status::Neutral, "1 (main only)");
                            } else {
                                let tooltip = worktrees
                                    .iter()
                                    .map(|worktree| {
                                        let branch =
                                            worktree.branch.as_deref().unwrap_or("(detached)");
                                        let tag = if worktree.is_main { " [main]" } else { "" };
                                        format!("{branch}{tag} — {}", worktree.path)
                                    })
                                    .collect::<Vec<_>>()
                                    .join("\n");
                                kit::status_chip(
                                    ui,
                                    kit::Status::Info,
                                    &worktrees.len().to_string(),
                                )
                                .on_hover_text(tooltip);
                            }
                        },
                    );
                    kit::row_divider(ui);
                    kit::setting_row(ui, "Provider", "The detected hosting service.", |ui| {
                        ui.label(format!("{:?}", caps.provider));
                    });
                }
                kit::row_divider(ui);
                kit::setting_row(ui, "Git", "The Git version used by Recorde.", |ui| {
                    ui.label(
                        self.git_readiness
                            .value
                            .as_ref()
                            .map(|value| value.summary())
                            .unwrap_or_else(|| "Checking system Git…".into()),
                    );
                });
            },
        );
        kit::section(
            ui,
            "Remotes",
            "Manage connections to other copies of this repository.",
            |ui| {
                kit::setting_row(ui, "Name", "A short name for this remote.", |ui| {
                    ui.text_edit_singleline(&mut self.repo_ui.remote_name_input);
                });
                kit::row_divider(ui);
                kit::setting_row(ui, "URL", "The address of the remote repository.", |ui| {
                    ui.text_edit_singleline(&mut self.repo_ui.origin_url_input);
                });
                kit::row_divider(ui);
                kit::setting_row(
                    ui,
                    "Add or edit remote",
                    "Save the name and URL above to this repository's Git configuration.",
                    |ui| {
                        ui.horizontal_wrapped(|ui| {
                            let label = if self.repo_ui.remote_rename_from.is_some() {
                                "Save remote"
                            } else {
                                "Add / update remote"
                            };
                            if ui
                                .add_enabled(
                                    !self.repo_ui.remote_name_input.trim().is_empty()
                                        && !self.repo_ui.origin_url_input.trim().is_empty(),
                                    kit::small_button(label),
                                )
                                .on_hover_text("Save this remote's name and URL.")
                                .on_disabled_hover_text("Enter both a remote name and a URL.")
                                .clicked()
                            {
                                let name = self.repo_ui.remote_name_input.trim().to_string();
                                let url = self.repo_ui.origin_url_input.trim().to_string();
                                let old = self.repo_ui.remote_rename_from.take();
                                let state = self.state.clone();
                                self.spawn_repo_settings_operation(path.clone(), move |path| {
                                    crate::git::remote::save_remote(
                                        path,
                                        &name,
                                        &url,
                                        old.as_deref(),
                                    )?;
                                    let mut s = state.lock().unwrap();
                                    if s.repo_path.as_deref() == Some(path) {
                                        s.invalidate_provider_data();
                                    }
                                    Ok(())
                                });
                            }
                            if ui
                                .add(kit::small_button("Clear"))
                                .on_hover_text(
                                    "Clear the remote fields without changing saved remotes.",
                                )
                                .clicked()
                            {
                                self.repo_ui.remote_name_input.clear();
                                self.repo_ui.origin_url_input.clear();
                                self.repo_ui.remote_rename_from = None;
                            }
                        });
                    },
                );
                kit::row_divider(ui);
                if snapshot.remotes.is_empty() {
                    kit::hint(ui, "No remotes configured.");
                }
                for (index, (name, url)) in snapshot.remotes.iter().enumerate() {
                    if index > 0 {
                        kit::row_divider(ui);
                    }
                    ui.push_id(name, |ui| {
                        kit::setting_row(ui, name, url, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                if ui
                                    .add(kit::small_button("Edit / rename"))
                                    .on_hover_text(
                                        "Load this remote into the fields above for editing.",
                                    )
                                    .clicked()
                                {
                                    self.repo_ui.remote_name_input = name.clone();
                                    self.repo_ui.remote_rename_from = Some(name.clone());
                                    self.repo_ui.origin_url_input = url.clone();
                                }
                                if ui
                                    .add(kit::danger_outline_button("Remove..."))
                                    .on_hover_text(
                                        "Review removing this remote and its local \
                                         tracking references.",
                                    )
                                    .clicked()
                                {
                                    self.repo_ui.remote_delete_pending = Some(name.clone());
                                }
                            });
                        });
                    });
                }

                // Retain the target only for the closing animation; actions still use
                // the live pending flag and are disabled as soon as it is cleared.
                let confirmation_id = ui.id().with(("remove_remote", &path));
                let removing = self.repo_ui.remote_delete_pending.is_some();
                if let Some(name) = &self.repo_ui.remote_delete_pending {
                    ui.data_mut(|data| data.insert_temp(confirmation_id, name.clone()));
                }
                let name = ui
                    .data_mut(|data| data.get_temp::<String>(confirmation_id).unwrap_or_default());
                motion::reveal(ui, confirmation_id, removing, |ui| {
                    ui.add_enabled_ui(removing, |ui| {
                        ui.add_space(kit::space::SM);
                        kit::callout(
                            ui,
                            kit::Status::Warning,
                            &format!("Remove remote {name} and its local tracking refs?"),
                        );
                        ui.add_space(kit::space::SM);
                        ui.horizontal_wrapped(|ui| {
                            if ui.add(kit::danger_button("Remove remote")).clicked() {
                                self.repo_ui.remote_delete_pending = None;
                                let state = self.state.clone();
                                self.spawn_repo_settings_operation(path.clone(), move |path| {
                                    crate::git::remote::remove_remote(path, &name)?;
                                    let mut s = state.lock().unwrap();
                                    if s.repo_path.as_deref() == Some(path) {
                                        s.invalidate_provider_data();
                                    }
                                    Ok(())
                                });
                            }
                            if ui.add(kit::secondary_button("Cancel")).clicked() {
                                self.repo_ui.remote_delete_pending = None;
                            }
                        });
                    });
                });
            },
        );
        kit::section(
            ui,
            "Repository identity and push settings",
            "Choose the identity and remotes used by this repository.",
            |ui| {
                kit::setting_row(
                    ui,
                    "Name",
                    "The name recorded on your commits in this repository.",
                    |ui| {
                        ui.text_edit_singleline(&mut self.repo_ui.repo_git_name_input);
                    },
                );
                kit::row_divider(ui);
                kit::setting_row(
                    ui,
                    "Email",
                    "The email recorded on your commits in this repository.",
                    |ui| {
                        ui.text_edit_singleline(&mut self.repo_ui.repo_git_email_input);
                    },
                );
                kit::row_divider(ui);
                kit::setting_row(
                    ui,
                    "Repository Git identity",
                    "Save this name and email for future commits in this repository.",
                    |ui| {
                        if ui
                            .add(kit::small_button("Save repository identity"))
                            .on_hover_text(
                                "Save the name and email for this repository. \
                                 Existing commits are unchanged.",
                            )
                            .clicked()
                        {
                            let name = self.repo_ui.repo_git_name_input.clone();
                            let email = self.repo_ui.repo_git_email_input.clone();
                            self.spawn_repo_settings_operation(path.clone(), move |path| {
                                save_git_identity(Some(path), &name, &email)
                            });
                        }
                    },
                );
                kit::row_divider(ui);
                self.repo_ui
                    .commit_signing
                    .show(ui, path.clone(), self.state.clone());
                kit::row_divider(ui);
                kit::setting_row(
                    ui,
                    "Provider remote",
                    "Use this remote's hosting service for pull requests, issues and releases.",
                    |ui| {
                        let provider_remote = snapshot.provider_remote.clone();
                        let mut chosen = provider_remote.clone();
                        egui::ComboBox::from_label("Provider remote")
                            .selected_text(if chosen.is_empty() {
                                "Choose a remote"
                            } else {
                                &chosen
                            })
                            .show_ui(ui, |ui| {
                                for (remote, _) in &snapshot.remotes {
                                    ui.selectable_value(&mut chosen, remote.clone(), remote);
                                }
                            });
                        if chosen != provider_remote {
                            let state = self.state.clone();
                            self.spawn_repo_settings_operation(path.clone(), move |path| {
                                crate::git::merge::run_git_seq(
                                    path,
                                    &["config", "--local", "recorde.providerRemote", &chosen],
                                )
                                .map_err(|e| e.user_message())?;
                                let mut s = state.lock().unwrap();
                                if s.repo_path.as_deref() == Some(path) {
                                    s.invalidate_provider_data();
                                }
                                Ok(())
                            });
                        }
                    },
                );
                kit::row_divider(ui);
                kit::setting_row(
                    ui,
                    "Default push remote",
                    "Send commits to this remote unless the current branch overrides it.",
                    |ui| {
                        let current = snapshot.push_default.clone();
                        let mut selected = current.clone();
                        egui::ComboBox::from_label("Default push remote")
                            .selected_text(if current.is_empty() {
                                "Choose a remote"
                            } else {
                                &current
                            })
                            .show_ui(ui, |ui| {
                                for (remote, _) in &snapshot.remotes {
                                    ui.selectable_value(&mut selected, remote.clone(), remote);
                                }
                            });
                        if selected != current {
                            self.spawn_repo_settings_operation(path.clone(), move |path| {
                                crate::git::process::git(
                                    path,
                                    &["config", "--local", "remote.pushDefault", &selected],
                                    &std::sync::atomic::AtomicBool::new(false),
                                )
                                .map(|_| ())
                                .map_err(|e| e.user_message())
                            });
                        }
                    },
                );
                if let Some(branch) = &snapshot.branch {
                    kit::row_divider(ui);
                    kit::setting_row(
                        ui,
                        &format!("Push remote for {branch}"),
                        "Override the push destination for this branch only.",
                        |ui| {
                            let key = format!("branch.{branch}.pushRemote");
                            let current = snapshot.branch_push_remote.clone();
                            let mut selected = current.clone();
                            egui::ComboBox::from_label(format!("Push remote for {branch}"))
                                .selected_text(if current.is_empty() {
                                    "Use repository default / upstream"
                                } else {
                                    &current
                                })
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut selected,
                                        String::new(),
                                        "Use repository default / upstream",
                                    );
                                    for (remote, _) in &snapshot.remotes {
                                        ui.selectable_value(&mut selected, remote.clone(), remote);
                                    }
                                });
                            if selected != current {
                                self.spawn_repo_settings_operation(path.clone(), move |path| {
                                    let args = if selected.is_empty() {
                                        vec!["config", "--local", "--unset", &key]
                                    } else {
                                        vec!["config", "--local", &key, &selected]
                                    };
                                    crate::git::merge::run_git_seq(path, &args)
                                        .map_err(|e| e.user_message())
                                });
                            }
                        },
                    );
                }
            },
        );
    }

    pub(super) fn render_about(&self, ui: &mut egui::Ui) {
        kit::page_header(
            ui,
            "About",
            "Learn about Recorde and its open-source components.",
        );
        kit::section(ui, "Info", "", |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.label("Recorde is built and maintained by ");
                ui.hyperlink_to("Antony Doyle", "https://antonydoyle.com")
                    .on_hover_text("https://antonydoyle.com");
                ui.label(".");
            });
            ui.add_space(kit::space::SM);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.label("If you like and use Recorde, please consider ");
                ui.hyperlink_to("buying me a coffee", "https://buymeacoffee.com/spaceant")
                    .on_hover_text("https://buymeacoffee.com/spaceant");
                ui.label(".");
            });
        });
        kit::section(ui, "Recorde", env!("CARGO_PKG_VERSION"), |ui| {
            kit::hint(ui, "A cross-platform desktop Git client built with Rust.");
            ui.add_space(kit::space::SM);
            ui.hyperlink_to(
                "https://github.com/antonydoyle/recorde",
                "https://github.com/antonydoyle/recorde",
            );
            kit::row_divider(ui);
            kit::hint(
                ui,
                "Robert Recorde (c. 1510 – 1558) was a Welsh physician and mathematician. \
                 He invented the equals sign (=) and also introduced the pre-existing \
                 plus (+) and minus (−) signs to English speakers in 1557.",
            );
            ui.add_space(kit::space::SM);
            kit::hint(
                ui,
                "Keyboard shortcuts are listed in the command palette and configurable \
                 in Settings > Keyboard. Ctrl on Windows/Linux is Command on macOS.",
            );
            kit::row_divider(ui);
            kit::disclosure(ui, "font_licenses", "Open-source licences", false, |ui| {
                for (name, license) in [
                    (
                        "Inter 4.1",
                        include_str!("../../assets/fonts/Inter-LICENSE.txt"),
                    ),
                    (
                        "Material Icons",
                        include_str!("../../assets/fonts/MaterialIcons-LICENSE.txt"),
                    ),
                    (
                        "Noto Sans Symbols",
                        include_str!("../../assets/fonts/NotoSansSymbols-OFL.txt"),
                    ),
                    (
                        "Noto Sans Symbols 2",
                        include_str!("../../assets/fonts/NotoSansSymbols2-OFL.txt"),
                    ),
                    (
                        "Noto Emoji",
                        include_str!("../../assets/fonts/NotoEmoji-OFL.txt"),
                    ),
                ] {
                    kit::disclosure(ui, name, name, false, |ui| {
                        kit::hint(ui, license);
                    });
                }
            });
        });
    }
}

/// Settings window width for a given screen width: 920 pixels, narrowing on
/// small screens but never below 640.
fn settings_width(screen_width: f32) -> f32 {
    (screen_width - 48.0).clamp(640.0, 920.0)
}
#[cfg(test)]
mod settings_tests {
    use super::category_matches;

    #[test]
    fn category_filter_matches_labels_and_keywords() {
        assert!(category_matches("Git", "pull rebase fetch", "PULL"));
        assert!(category_matches(
            "Appearance",
            "theme motion",
            "appearance motion"
        ));
        assert!(!category_matches("Git", "pull fetch", "editor"));
        assert!(category_matches("Git", "pull fetch", "  "));
    }

    #[test]
    fn git_retry_button_clears_failure_without_a_repository() {
        let ctx = egui::Context::default();
        let mut monitor = crate::git::readiness::Monitor::default();
        monitor.value = Some(crate::git::readiness::Readiness {
            problem: Some("System Git was not found. Retry Git check.".into()),
            ..Default::default()
        });
        let mut frame = |events| {
            ctx.begin_pass(egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 600.0),
                )),
                events,
                ..Default::default()
            });
            egui::CentralPanel::default().show(
                &mut egui::Ui::new(
                    ctx.clone(),
                    egui::Id::new("git_readiness_test_root"),
                    egui::UiBuilder::new().max_rect(ctx.content_rect()),
                ),
                |ui| super::git_readiness_status(ui, &mut monitor),
            );
            let mut output = ctx.end_pass();
            output.textures_delta.clear();
            output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "Retry Git check" => {
                    Some(text.galley.rect.translate(text.pos.to_vec2()).center())
                }
                _ => None,
            })
        };
        let position = frame(vec![]).expect("Retry is available without a repository");
        let button = |pressed| egui::Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        frame(vec![egui::Event::PointerMoved(position), button(true)]);
        frame(vec![button(false)]);
        assert!(monitor.value.is_none());
        assert!(monitor.checking());
    }
}
