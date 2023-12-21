// Copyright 2023 System76 <info@system76.com>
// SPDX-License-Identifier: GPL-3.0-only

use alacritty_terminal::{
    config::Config as TermConfig, event::Event as TermEvent, term::color::Colors as TermColors, tty,
};
use cosmic::{
    app::{Command, Core, Settings},
    cosmic_theme, executor,
    iced::{
        futures::SinkExt,
        subscription::{self, Subscription},
        widget::row,
        window, Alignment, Length,
    },
    iced_core::Size,
    style,
    widget::{self, segmented_button},
    ApplicationExt, Element,
};
use std::{any::TypeId, collections::HashMap, sync::Mutex};
use tokio::sync::mpsc;

use self::terminal::{Terminal, TerminalScroll};
mod terminal;

use self::terminal_box::terminal_box;
mod terminal_box;

mod terminal_theme;

/// Runs application with these settings
#[rustfmt::skip]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    // Set up environmental variables for terminal
    let mut term_config = TermConfig::default();
    // Override TERM for better compatibility
    term_config.env.insert("TERM".to_string(), "xterm-256color".to_string());
    tty::setup_env(&term_config);

    let settings = Settings::default()
        .antialiasing(true)
        .client_decorations(true)
        .debug(false)
        .default_icon_theme("Cosmic")
        .default_text_size(16.0)
        .scale_factor(1.0)
        .size(Size::new(1024., 768.))
        .theme(cosmic::Theme::dark());

    cosmic::app::run::<App>(settings, term_config)?;

    Ok(())
}

/// Messages that are used specifically by our [`App`].
#[derive(Clone, Debug)]
pub enum Message {
    TabActivate(segmented_button::Entity),
    TabClose(segmented_button::Entity),
    TabNew,
    TermEvent(segmented_button::Entity, TermEvent),
    TermEventTx(mpsc::Sender<(segmented_button::Entity, TermEvent)>),
}

/// The [`App`] stores application-specific state.
pub struct App {
    core: Core,
    tab_model: segmented_button::Model<segmented_button::SingleSelect>,
    term_event_tx_opt: Option<mpsc::Sender<(segmented_button::Entity, TermEvent)>>,
    term_config: TermConfig,
    terminal_theme: String,
    terminal_themes: HashMap<String, TermColors>,
}

/// Implement [`cosmic::Application`] to integrate with COSMIC.
impl cosmic::Application for App {
    /// Default async executor to use with the app.
    type Executor = executor::Default;

    /// Argument received
    type Flags = TermConfig;

    /// Message type specific to our [`App`].
    type Message = Message;

    /// The unique application ID to supply to the window manager.
    const APP_ID: &'static str = "org.cosmic.AppDemo";

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    /// Creates the application, and optionally emits command on initialize.
    fn init(mut core: Core, term_config: Self::Flags) -> (Self, Command<Self::Message>) {
        core.window.content_container = false;

        let mut app = App {
            core,
            tab_model: segmented_button::ModelBuilder::default().build(),
            term_event_tx_opt: None,
            term_config,
            terminal_theme: "OneHalfDark".to_string(),
            terminal_themes: terminal_theme::terminal_themes(),
        };

        let command = app.update_title();

        (app, command)
    }

    /// Handle application events here.
    fn update(&mut self, message: Self::Message) -> Command<Self::Message> {
        match message {
            Message::TabActivate(entity) => {
                self.tab_model.activate(entity);
                return self.update_title();
            }
            Message::TabClose(entity) => {
                // Activate closest item
                if let Some(position) = self.tab_model.position(entity) {
                    if position > 0 {
                        self.tab_model.activate_position(position - 1);
                    } else {
                        self.tab_model.activate_position(position + 1);
                    }
                }

                // Remove item
                self.tab_model.remove(entity);

                // If that was the last tab, close window
                if self.tab_model.iter().next().is_none() {
                    return window::close(window::Id::MAIN);
                }

                return self.update_title();
            }
            Message::TabNew => match &self.term_event_tx_opt {
                Some(term_event_tx) => match self.terminal_themes.get(&self.terminal_theme) {
                    Some(colors) => {
                        let entity = self
                            .tab_model
                            .insert()
                            .text("New Terminal")
                            .closable()
                            .activate()
                            .id();
                        let terminal = Terminal::new(
                            entity,
                            term_event_tx.clone(),
                            &self.term_config,
                            colors.clone(),
                        );
                        self.tab_model
                            .data_set::<Mutex<Terminal>>(entity, Mutex::new(terminal));
                    }
                    None => {
                        log::error!("failed to find terminal theme {:?}", self.terminal_theme);
                    }
                },
                None => {
                    log::warn!("tried to create new tab before having event channel");
                }
            },
            Message::TermEvent(entity, event) => match event {
                TermEvent::Bell => {
                    //TODO: audible or visible bell options?
                }
                TermEvent::ColorRequest(index, f) => {
                    if let Some(terminal) = self.tab_model.data::<Mutex<Terminal>>(entity) {
                        let terminal = terminal.lock().unwrap();
                        let rgb = terminal.colors()[index].unwrap_or_default();
                        let text = f(rgb);
                        terminal.input_no_scroll(text.into_bytes());
                    }
                }
                TermEvent::Exit => {
                    return self.update(Message::TabClose(entity));
                }
                TermEvent::PtyWrite(text) => {
                    if let Some(terminal) = self.tab_model.data::<Mutex<Terminal>>(entity) {
                        let terminal = terminal.lock().unwrap();
                        terminal.input_no_scroll(text.into_bytes());
                    }
                }
                TermEvent::ResetTitle => {
                    self.tab_model.text_set(entity, "New Terminal");
                    return self.update_title();
                }
                TermEvent::TextAreaSizeRequest(f) => {
                    if let Some(terminal) = self.tab_model.data::<Mutex<Terminal>>(entity) {
                        let terminal = terminal.lock().unwrap();
                        let text = f(terminal.size().into());
                        terminal.input_no_scroll(text.into_bytes());
                    }
                }
                TermEvent::Title(title) => {
                    self.tab_model.text_set(entity, title);
                    return self.update_title();
                }
                TermEvent::MouseCursorDirty | TermEvent::Wakeup => {
                    if let Some(terminal) = self.tab_model.data::<Mutex<Terminal>>(entity) {
                        let mut terminal = terminal.lock().unwrap();
                        terminal.update();
                    }
                }
                _ => {
                    println!("TODO: {:?}", event);
                }
            },
            Message::TermEventTx(term_event_tx) => {
                self.term_event_tx_opt = Some(term_event_tx);
            }
        }

        Command::none()
    }

    fn header_start(&self) -> Vec<Element<Self::Message>> {
        let cosmic_theme::Spacing { space_xxs, .. } = self.core().system_theme().cosmic().spacing;

        vec![row![
            widget::button(widget::icon::from_name("list-add-symbolic").size(16).icon())
                .on_press(Message::TabNew)
                .padding(space_xxs)
                .style(style::Button::Icon)
        ]
        .align_items(Alignment::Center)
        .into()]
    }

    /// Creates a view after each update.
    fn view(&self) -> Element<Self::Message> {
        let cosmic_theme::Spacing { space_xxs, .. } = self.core().system_theme().cosmic().spacing;

        let mut tab_column = widget::column::with_capacity(1);

        if self.tab_model.iter().count() > 1 {
            tab_column = tab_column.push(
                widget::container(row![
                    widget::view_switcher::horizontal(&self.tab_model)
                        .button_height(32)
                        .button_spacing(space_xxs)
                        .on_activate(Message::TabActivate)
                        .on_close(Message::TabClose)
                        .width(Length::Shrink),
                    widget::horizontal_space(Length::Fill)
                ])
                .style(style::Container::Background),
            );
        }

        match self
            .tab_model
            .data::<Mutex<Terminal>>(self.tab_model.active())
        {
            Some(terminal) => {
                //TODO
                tab_column = tab_column.push(terminal_box(terminal));
            }
            None => {
                //TODO
            }
        }

        let content: Element<_> = tab_column.into();

        // Uncomment to debug layout:
        //content.explain(cosmic::iced::Color::WHITE)
        content
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        struct TerminalEventWorker;
        subscription::channel(
            TypeId::of::<TerminalEventWorker>(),
            100,
            |mut output| async move {
                let (event_tx, mut event_rx) = mpsc::channel(100);
                output.send(Message::TermEventTx(event_tx)).await.unwrap();

                // Create first terminal tab
                output.send(Message::TabNew).await.unwrap();

                while let Some((entity, event)) = event_rx.recv().await {
                    output
                        .send(Message::TermEvent(entity, event))
                        .await
                        .unwrap();
                }

                panic!("terminal event channel closed");
            },
        )
    }
}

impl App
where
    Self: cosmic::Application,
{
    fn update_title(&mut self) -> Command<Message> {
        let (header_title, window_title) = match self.tab_model.text(self.tab_model.active()) {
            Some(tab_title) => (
                tab_title.to_string(),
                format!("{tab_title} — COSMIC Terminal"),
            ),
            None => (String::new(), "COSMIC Terminal".to_string()),
        };
        self.set_header_title(header_title);
        self.set_window_title(window_title)
    }
}
