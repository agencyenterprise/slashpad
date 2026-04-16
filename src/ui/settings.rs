//! Settings panel — API key + hotkey display.

use iced::widget::{button, checkbox, column, container, pick_list, row, text, text_input, Column};
use iced::{Element, Length};

use crate::app::{Message, UpdateStatus};
use crate::settings::PreferredTerminal;

#[allow(clippy::too_many_arguments)]
pub fn view<'a>(
    api_key_input: &'a str,
    api_key_visible: bool,
    use_subscription: bool,
    current_hotkey: &'a str,
    recording: bool,
    hotkey_error: Option<&'a str>,
    preferred_terminal: PreferredTerminal,
    load_user_settings: bool,
    update_status: &UpdateStatus,
    launch_at_login: bool,
    is_app_bundle: bool,
) -> Element<'a, Message> {
    let mut title = row![
        text("/slashpad").size(15).color(super::theme::ACCENT),
        text(format!("  v{}", env!("CARGO_PKG_VERSION")))
            .size(11)
            .color(super::theme::MUTED),
    ]
    .spacing(0)
    .align_y(iced::Alignment::Center);

    match update_status {
        UpdateStatus::Idle | UpdateStatus::Checking | UpdateStatus::UpToDate => {
            title = title.push(
                text("  Latest version")
                    .size(11)
                    .color(super::theme::MUTED),
            );
        }
        UpdateStatus::Available { version, .. } => {
            title = title.push(
                button(
                    text(format!("  Upgrade to v{version}"))
                        .size(11)
                        .color(super::theme::ACCENT),
                )
                .on_press(Message::UpgradeClicked)
                .padding(0)
                .style(|_, status| {
                    let text_color = match status {
                        iced::widget::button::Status::Hovered => super::theme::TEXT,
                        _ => super::theme::ACCENT,
                    };
                    iced::widget::button::Style {
                        background: None,
                        text_color,
                        ..Default::default()
                    }
                }),
            );
        }
        UpdateStatus::Upgrading => {
            title = title.push(
                text("  Upgrading...")
                    .size(11)
                    .color(super::theme::MUTED),
            );
        }
    }

    title = title.push(iced::widget::horizontal_space());
    title = title.push(
        button(text("esc").size(11).color(super::theme::MUTED))
            .on_press(Message::CloseSettings)
            .padding(0)
            .style(|_, _| iced::widget::button::Style {
                background: None,
                text_color: super::theme::MUTED,
                ..Default::default()
            }),
    );

    // Inner text_input: transparent so it adopts the surrounding
    // container's background, themed to match the rest of the app.
    let input = text_input("sk-ant-...", api_key_input)
        .on_input(Message::ApiKeyInputChanged)
        .secure(!api_key_visible)
        .padding(8)
        .size(13)
        .width(Length::Fill)
        .style(|_theme: &iced::Theme, _status| iced::widget::text_input::Style {
            background: iced::Background::Color(iced::Color::TRANSPARENT),
            border: iced::Border {
                color: iced::Color::TRANSPARENT,
                width: 0.0,
                radius: 0.0.into(),
            },
            icon: super::theme::MUTED,
            placeholder: super::theme::MUTED,
            value: super::theme::TEXT,
            selection: iced::Color {
                a: 0.35,
                ..super::theme::ACCENT
            },
        });

    let toggle_label = if api_key_visible { "Hide" } else { "Show" };
    let toggle_btn = button(text(toggle_label).size(11).color(super::theme::MUTED))
        .on_press(Message::ToggleApiKeyVisibility)
        .padding([4, 8])
        .style(|_, status| {
            let text_color = match status {
                iced::widget::button::Status::Hovered => super::theme::TEXT,
                _ => super::theme::MUTED,
            };
            iced::widget::button::Style {
                background: None,
                text_color,
                border: iced::Border {
                    color: iced::Color::TRANSPARENT,
                    width: 0.0,
                    radius: 6.0.into(),
                },
                ..Default::default()
            }
        });

    // Build the inline controls row. The clear (×) button only shows up
    // when there is something to clear.
    let mut inner = row![input, toggle_btn]
        .spacing(2)
        .align_y(iced::Alignment::Center);
    if !api_key_input.is_empty() {
        let clear_btn = button(text("×").size(14).color(super::theme::MUTED))
            .on_press(Message::ClearApiKey)
            .padding([2, 8])
            .style(|_, status| {
                let text_color = match status {
                    iced::widget::button::Status::Hovered => super::theme::DANGER,
                    _ => super::theme::MUTED,
                };
                iced::widget::button::Style {
                    background: None,
                    text_color,
                    border: iced::Border {
                        color: iced::Color::TRANSPARENT,
                        width: 0.0,
                        radius: 6.0.into(),
                    },
                    ..Default::default()
                }
            });
        inner = inner.push(clear_btn);
    }

    // Wrap the input + side buttons in a bordered rounded container so
    // the whole thing reads as one control.
    let input_shell = container(inner)
        .padding([0, 4])
        .width(Length::Fill)
        .style(|_theme: &iced::Theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(super::theme::SURFACE_2)),
            border: iced::Border {
                color: super::theme::SURFACE_3,
                width: 1.0,
                radius: 8.0.into(),
            },
            text_color: Some(super::theme::TEXT),
            ..Default::default()
        });

    let subscription_checkbox = checkbox("Use Claude subscription", use_subscription)
        .on_toggle(Message::UseSubscriptionToggled)
        .size(14)
        .text_size(13)
        .spacing(8)
        .style(|_theme: &iced::Theme, status| {
            let checked = matches!(
                status,
                iced::widget::checkbox::Status::Active { is_checked: true }
                    | iced::widget::checkbox::Status::Hovered { is_checked: true }
                    | iced::widget::checkbox::Status::Disabled { is_checked: true }
            );
            iced::widget::checkbox::Style {
                background: iced::Background::Color(if checked {
                    super::theme::ACCENT
                } else {
                    super::theme::SURFACE_2
                }),
                icon_color: super::theme::SURFACE_0,
                border: iced::Border {
                    color: if checked {
                        super::theme::ACCENT
                    } else {
                        super::theme::SURFACE_3
                    },
                    width: 1.0,
                    radius: 4.0.into(),
                },
                text_color: Some(super::theme::TEXT),
            }
        });

    let mut api_row: Column<'_, Message> = column![subscription_checkbox].spacing(10);
    if use_subscription {
        api_row = api_row.push(
            text("Authenticated via `claude login` in your terminal")
                .size(11)
                .color(super::theme::MUTED),
        );
    } else {
        api_row = api_row.push(
            column![
                text("Anthropic API Key")
                    .size(11)
                    .color(super::theme::MUTED),
                input_shell,
            ]
            .spacing(6),
        );
    }

    let hotkey_label_text = if recording {
        "Press a shortcut…".to_string()
    } else {
        current_hotkey.to_string()
    };
    let hotkey_label_color = if recording {
        super::theme::MUTED
    } else {
        super::theme::TEXT
    };
    let hotkey_on_press = if recording {
        Message::CancelRecordHotkey
    } else {
        Message::StartRecordHotkey
    };

    let hotkey_button = button(text(hotkey_label_text).size(13).color(hotkey_label_color))
        .on_press(hotkey_on_press)
        .padding([8, 14])
        .style(|_, _| iced::widget::button::Style {
            background: Some(iced::Background::Color(super::theme::SURFACE_2)),
            text_color: super::theme::TEXT,
            border: iced::Border {
                color: super::theme::SURFACE_3,
                width: 1.0,
                radius: 8.0.into(),
            },
            ..Default::default()
        });

    let mut hotkey_row = column![
        text("Global Hotkey").size(11).color(super::theme::MUTED),
        hotkey_button,
    ]
    .spacing(6);

    if let Some(err) = hotkey_error {
        hotkey_row = hotkey_row.push(text(err.to_string()).size(11).color(super::theme::DANGER));
    }

    let terminal_picker = pick_list(
        &PreferredTerminal::ALL[..],
        Some(preferred_terminal),
        Message::PreferredTerminalChanged,
    )
    .text_size(13)
    .padding([8, 14])
    .style(|_theme: &iced::Theme, _status| iced::widget::pick_list::Style {
        text_color: super::theme::TEXT,
        placeholder_color: super::theme::MUTED,
        handle_color: super::theme::MUTED,
        background: iced::Background::Color(super::theme::SURFACE_2),
        border: iced::Border {
            color: super::theme::SURFACE_3,
            width: 1.0,
            radius: 8.0.into(),
        },
    });

    let terminal_row = column![
        text("Preferred Terminal").size(11).color(super::theme::MUTED),
        terminal_picker,
    ]
    .spacing(6);

    let user_settings_checkbox =
        checkbox("Load user-level Claude settings & skills", load_user_settings)
            .on_toggle(Message::LoadUserSettingsToggled)
            .size(14)
            .text_size(13)
            .spacing(8)
            .style(|_theme: &iced::Theme, status| {
                let checked = matches!(
                    status,
                    iced::widget::checkbox::Status::Active { is_checked: true }
                        | iced::widget::checkbox::Status::Hovered { is_checked: true }
                        | iced::widget::checkbox::Status::Disabled { is_checked: true }
                );
                iced::widget::checkbox::Style {
                    background: iced::Background::Color(if checked {
                        super::theme::ACCENT
                    } else {
                        super::theme::SURFACE_2
                    }),
                    icon_color: super::theme::SURFACE_0,
                    border: iced::Border {
                        color: if checked {
                            super::theme::ACCENT
                        } else {
                            super::theme::SURFACE_3
                        },
                        width: 1.0,
                        radius: 4.0.into(),
                    },
                    text_color: Some(super::theme::TEXT),
                }
            });

    let user_settings_row = column![
        user_settings_checkbox,
        text("Loads CLAUDE.md, skills, and hooks from ~/.claude/")
            .size(11)
            .color(super::theme::MUTED),
    ]
    .spacing(6);

    // "Launch at login" — only shown for .app bundle installs.
    // Homebrew users use `brew services` instead.
    let mut body_items: Vec<Element<'a, Message>> = vec![
        title.into(),
        api_row.into(),
        hotkey_row.into(),
        terminal_row.into(),
        user_settings_row.into(),
    ];

    if is_app_bundle {
        let login_checkbox =
            checkbox("Launch at login", launch_at_login)
                .on_toggle(Message::LaunchAtLoginToggled)
                .size(14)
                .text_size(13)
                .spacing(8)
                .style(|_theme: &iced::Theme, status| {
                    let checked = matches!(
                        status,
                        iced::widget::checkbox::Status::Active { is_checked: true }
                            | iced::widget::checkbox::Status::Hovered { is_checked: true }
                            | iced::widget::checkbox::Status::Disabled { is_checked: true }
                    );
                    iced::widget::checkbox::Style {
                        background: iced::Background::Color(if checked {
                            super::theme::ACCENT
                        } else {
                            super::theme::SURFACE_2
                        }),
                        icon_color: super::theme::SURFACE_0,
                        border: iced::Border {
                            color: if checked {
                                super::theme::ACCENT
                            } else {
                                super::theme::SURFACE_3
                            },
                            width: 1.0,
                            radius: 4.0.into(),
                        },
                        text_color: Some(super::theme::TEXT),
                    }
                });
        body_items.push(login_checkbox.into());
    }

    let actions = row![
        button(text("Show Launcher").size(12).color(super::theme::TEXT))
            .on_press(Message::HotkeyPressed)
            .padding([8, 14])
            .style(|_, _| iced::widget::button::Style {
                background: Some(iced::Background::Color(super::theme::SURFACE_2)),
                text_color: super::theme::TEXT,
                border: iced::Border {
                    color: super::theme::SURFACE_3,
                    width: 1.0,
                    radius: 8.0.into(),
                },
                ..Default::default()
            }),
        iced::widget::horizontal_space(),
        button(text("Quit Slashpad").size(12).color(super::theme::TEXT))
            .on_press(Message::QuitRequested)
            .padding([8, 14])
            .style(|_, _| iced::widget::button::Style {
                background: Some(iced::Background::Color(super::theme::SURFACE_2)),
                text_color: super::theme::TEXT,
                border: iced::Border {
                    color: super::theme::SURFACE_3,
                    width: 1.0,
                    radius: 8.0.into(),
                },
                ..Default::default()
            }),
    ]
    .spacing(8);

    body_items.push(actions.into());

    let body = Column::with_children(body_items)
        .spacing(14)
        .padding(16);

    container(body)
        .width(Length::Fill)
        .style(|_theme: &iced::Theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(super::theme::SURFACE_1)),
            border: iced::Border {
                color: super::theme::SURFACE_3,
                width: 1.0,
                radius: 12.0.into(),
            },
            text_color: Some(super::theme::TEXT),
            ..Default::default()
        })
        .into()
}
