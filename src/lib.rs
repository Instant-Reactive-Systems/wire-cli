mod tui;

use futures::{SinkExt, StreamExt};
use std::collections::VecDeque;
use tokio::sync::mpsc::{Receiver, Sender};
pub use tui::Tui;

#[cfg(not(all(
    any(feature = "out-json", feature = "out-ron"),
    any(feature = "in-json", feature = "in-ron")
)))]
compile_error!("need at least one input and one output feature-flag enabled");

/// The type for incoming messages.
pub type Res<Event, Err> = std::result::Result<wire::TimestampedEvent<Event>, Err>;

/// The maximum number of messages in the message history buffer.
const MAX_MESSAGES: usize = 100;

/// The state of the app.
enum State {
    InputSelected,
    MsgListSelected,
}

/// Configures the client externally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientCfg {
    pub url: String,
}

/// A client that starts a TUI app for communicating with a server
/// and sending it requests.
pub struct Client<Action, Event, Err>
where
    Action: serde::Serialize + serde::de::DeserializeOwned + std::fmt::Debug + Send + 'static,
    Event: serde::de::DeserializeOwned + std::fmt::Debug + Send + 'static,
    Err: serde::de::DeserializeOwned + std::fmt::Debug + Send + 'static,
{
    cfg: ClientCfg,
    input: String,
    msgs: VecDeque<String>,
    state: State,
    scroll_state: ratatui::widgets::ListState,
    _phant: std::marker::PhantomData<(Action, Event, Err)>,
}

impl<Action, Event, Err> Client<Action, Event, Err>
where
    Action: serde::Serialize + serde::de::DeserializeOwned + std::fmt::Debug + Send + 'static,
    Event: serde::de::DeserializeOwned + std::fmt::Debug + Send + 'static,
    Err: serde::de::DeserializeOwned + std::fmt::Debug + Send + 'static,
{
    /// Creates a new client.
    pub fn new(cfg: ClientCfg) -> Self {
        Self {
            cfg,
            input: Default::default(),
            msgs: Default::default(),
            state: State::InputSelected,
            scroll_state: Default::default(),
            _phant: Default::default(),
        }
    }

    /// Starts the client.
    pub async fn start(self) -> color_eyre::Result<()> {
        color_eyre::install()?;

        let (stream, _res) = tokio_tungstenite::connect_async(self.cfg.url.clone()).await?;
        let (mut ws_tx, mut ws_rx) = stream.split();
        let (res_tx, res_rx) = tokio::sync::mpsc::channel::<Res<Event, Err>>(100);
        let (req_tx, mut req_rx) = tokio::sync::mpsc::channel::<Action>(100);
        let (sys_tx, sys_rx) = tokio::sync::mpsc::channel::<String>(100);

        // read ws messages
        let read_ws_task =
            tokio::spawn({
                let sys_tx = sys_tx.clone();

                async move {
                    while let Some(msg) = ws_rx.next().await {
                        match msg {
                            Ok(msg) => {
                                match msg {
                                    tokio_tungstenite::tungstenite::Message::Text(text) => {
                                        // TODO: support more message formats
                                        #[cfg(feature = "out-json")]
                                        let parse_res =
                                            serde_json::from_str::<Res<Event, Err>>(&text);
                                        #[cfg(feature = "out-ron")]
                                        let parse_res = ron::from_str::<Res<Event, Err>>(&text);
                                        #[cfg(not(any(feature = "out-json", feature = "out-ron")))]
                                        let parse_res: Result<String, String> = unreachable!();

                                        let res = match parse_res {
                                            Ok(res) => res,
                                            Err(err) => {
                                                if let Err(_) = sys_tx.send(err.to_string()).await {
                                                    break;
                                                }
                                                continue;
                                            }
                                        };
                                        #[cfg(any(feature = "out-json", feature = "out-ron"))]
                                        if let Err(_err) = res_tx.send(res).await {
                                            break;
                                        };
                                    }
                                    _ => {}
                                }
                            }
                            Err(err) => {
                                if let Err(_) = sys_tx.send(err.to_string()).await {
                                    break;
                                }
                                continue;
                            }
                        }
                    }
                }
            });

        // write ws messages
        let write_ws_task = tokio::spawn(async move {
            while let Some(req) = req_rx.recv().await {
                #[cfg(feature = "out-json")]
                let serialized = serde_json::to_string(&req);
                #[cfg(feature = "out-ron")]
                let serialized = ron::to_string(&req);
                #[cfg(not(any(feature = "out-json", feature = "out-ron")))]
                let serialized: Result<String, String> = unreachable!();

                let msg = match serialized {
                    Ok(msg) => msg,
                    Err(err) => {
                        if let Err(_) = sys_tx.send(err.to_string()).await {
                            break;
                        }
                        continue;
                    }
                };
                if let Err(_err) = ws_tx
                    .send(tokio_tungstenite::tungstenite::Message::Text(msg.into()))
                    .await
                {
                    break;
                }
            }
        });

        let mut tui = tui::Tui::new()?
            .tick_rate(4.0) // 4 ticks per second
            .frame_rate(30.0); // 30 frames per second
        tui.enter()?;
        let result = self.run(&mut tui, req_tx, res_rx, sys_rx).await;
        tui.exit()?;

        read_ws_task.abort();
        write_ws_task.abort();

        result
    }

    async fn run(
        mut self,
        tui: &mut Tui,
        req_tx: Sender<Action>,
        mut res_rx: Receiver<Res<Event, Err>>,
        mut sys_rx: Receiver<String>,
    ) -> color_eyre::Result<()> {
        loop {
            self.render(tui)?;

            while let Ok(res) = res_rx.try_recv() {
                match res {
                    Ok(res) => {
                        self.add_msg(format!("received: {:?}", res));
                    }
                    Err(err) => {
                        self.add_msg(format!("error: {:?}", err));
                    }
                }
            }

            while let Ok(msg) = sys_rx.try_recv() {
                self.add_msg(format!("internal message: {}", msg));
            }

            if let Some(evt) = tui.next().await {
                match self.state {
                    State::InputSelected => match evt {
                        tui::Event::Key(evt) => match evt.code {
                            crossterm::event::KeyCode::Esc => break Ok(()),
                            crossterm::event::KeyCode::Tab => self.state = State::MsgListSelected,
                            crossterm::event::KeyCode::Enter => {
                                if !self.input.is_empty() {
                                    self.add_msg(format!("sent: {}", self.input.clone()));
                                    #[cfg(feature = "in-json")]
                                    let parse_res = serde_json::from_str::<Action>(&self.input);
                                    #[cfg(feature = "in-ron")]
                                    let parse_res = ron::from_str::<Action>(&self.input);
                                    #[cfg(not(any(feature = "in-json", feature = "in-ron")))]
                                    let parse_res: Result<
                                        Res<Event, Err>,
                                        String,
                                    > = unreachable!();

                                    let Ok(req) = parse_res else {
                                        self.add_msg(format!(
                                            "error: invalid request format: {}",
                                            self.input
                                        ));
                                        self.input.clear();
                                        continue;
                                    };
                                    #[cfg(any(feature = "in-json", feature = "in-ron"))]
                                    req_tx
                                        .send(req)
                                        .await
                                        .expect("request channel should not be closed");
                                    self.input.clear();
                                }
                            }
                            crossterm::event::KeyCode::Backspace => _ = self.input.pop(),
                            crossterm::event::KeyCode::Char(ch) => self.input.push(ch),
                            _ => {}
                        },
                        _ => {}
                    },
                    State::MsgListSelected => match evt {
                        tui::Event::Key(evt) => match evt.code {
                            crossterm::event::KeyCode::Esc => break Ok(()),
                            crossterm::event::KeyCode::Backspace => {
                                if evt
                                    .modifiers
                                    .contains(crossterm::event::KeyModifiers::SHIFT)
                                {
                                    self.input.clear();
                                    self.add_msg("cleared input box".to_string());
                                }
                            }
                            crossterm::event::KeyCode::Tab => self.state = State::InputSelected,
                            crossterm::event::KeyCode::Char('j') => {
                                match self.scroll_state.selected() {
                                    Some(idx) => {
                                        if idx < self.msgs.len().saturating_sub(1) {
                                            self.scroll_state.select(Some(idx + 1));
                                        }
                                    }
                                    None => {
                                        if !self.msgs.is_empty() {
                                            self.scroll_state.select(Some(0))
                                        }
                                    }
                                }
                            }
                            crossterm::event::KeyCode::Char('k') => {
                                match self.scroll_state.selected() {
                                    Some(idx) => {
                                        if idx > 0 {
                                            self.scroll_state.select(Some(idx - 1));
                                        }
                                    }
                                    None => {
                                        if !self.msgs.is_empty() {
                                            self.scroll_state.select(Some(0))
                                        }
                                    }
                                }
                            }
                            _ => {}
                        },
                        _ => {}
                    },
                }
            }
        }
    }

    fn render(&mut self, tui: &mut Tui) -> color_eyre::Result<()> {
        tui.draw(move |f: &mut ratatui::Frame| {
            // wedge
            let [help_area, input_area, msgs_area] = ratatui::layout::Layout::vertical([
                ratatui::layout::Constraint::Length(1),
                ratatui::layout::Constraint::Length(3),
                ratatui::layout::Constraint::Min(1),
            ])
            .areas(f.area());

            let widget = ratatui::widgets::Paragraph::new(match self.state {
                State::InputSelected => format!("In INPUT mode"),
                State::MsgListSelected => format!(
                    "In VIEW mode | Selected {:?} message",
                    self.scroll_state.selected()
                ),
            });
            f.render_widget(widget, help_area);

            let block = {
                let block = ratatui::widgets::Block::default()
                    .borders(ratatui::widgets::Borders::all())
                    .title("Input");
                let block = if matches!(self.state, State::InputSelected) {
                    block.border_style(
                        ratatui::style::Style::default().fg(ratatui::style::Color::Yellow),
                    )
                } else {
                    block
                };

                block
            };
            let widget = ratatui::widgets::Paragraph::new(self.input.clone()).block(block);
            f.render_widget(widget, input_area);

            let block = {
                let block = ratatui::widgets::Block::default()
                    .borders(ratatui::widgets::Borders::all())
                    .title("Events");
                let block = if matches!(self.state, State::MsgListSelected) {
                    block.border_style(
                        ratatui::style::Style::default().fg(ratatui::style::Color::Yellow),
                    )
                } else {
                    block
                };

                block
            };
            let msgs = self
                .msgs
                .iter()
                .cloned()
                .map(|msg| ratatui::widgets::ListItem::new(msg));
            let widget = ratatui::widgets::List::new(msgs)
                .block(block)
                .highlight_style(
                    ratatui::style::Style::default().fg(ratatui::style::Color::Yellow),
                );
            f.render_stateful_widget(widget, msgs_area, &mut self.scroll_state);
        })?;

        Ok(())
    }
}

impl<Req, Res, Err> Client<Req, Res, Err>
where
    Req: serde::Serialize + serde::de::DeserializeOwned + std::fmt::Debug + Send + 'static,
    Res: serde::de::DeserializeOwned + std::fmt::Debug + Send + 'static,
    Err: serde::de::DeserializeOwned + std::fmt::Debug + Send + 'static,
{
    fn add_msg(&mut self, msg: String) {
        if self.msgs.len() >= MAX_MESSAGES {
            self.msgs.pop_front();
        }

        self.msgs.push_back(msg);
    }
}
