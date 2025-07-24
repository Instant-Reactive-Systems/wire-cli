mod tui;

use std::collections::VecDeque;
pub use tui::{Tui, Event};

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
pub struct Client<Req, Res, Err> {
	tx: (),
	rx: (),
	input: String,
	msgs: VecDeque<String>,
	state: State,
	scroll_state: ratatui::widgets::ListState,
	_phant: std::marker::PhantomData<(Req, Res, Err)>,
}

impl<Req, Res, Err> Client<Req, Res, Err> {
	/// Creates a new client.
	pub fn new(cfg: ClientCfg) -> Self {
		Self {
			tx: (),
			rx: (),
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
		let mut tui = tui::Tui::new()?
			.tick_rate(4.0) // 4 ticks per second
			.frame_rate(30.0); // 30 frames per second
		tui.enter()?;
		let result = self.run(&mut tui).await;
		tui.exit()?;

		result
	}

	async fn run(mut self, tui: &mut Tui) -> color_eyre::Result<()> {
		loop {
			self.render(tui)?;

			if let Some(evt) = tui.next().await {
				match self.state {
					State::InputSelected => match evt {
						Event::Key(evt) => match evt.code {
							crossterm::event::KeyCode::Esc => break Ok(()),
							crossterm::event::KeyCode::Tab => self.state = State::MsgListSelected,
							crossterm::event::KeyCode::Enter => {
								if !self.input.is_empty() {
									self.add_msg(format!("sent: {}", self.input.clone()));
									// self.stream.send(self.input.clone());
									self.input.clear();
								}
							},
							crossterm::event::KeyCode::Backspace => _ = self.input.pop(),
							crossterm::event::KeyCode::Char(ch) => self.input.push(ch),
							_ => {},
						},
						_ => {},
					},
					State::MsgListSelected => match evt {
						Event::Key(evt) => match evt.code {
							crossterm::event::KeyCode::Esc => break Ok(()),
							crossterm::event::KeyCode::Tab => self.state = State::InputSelected,
							crossterm::event::KeyCode::Char('j') => match self.scroll_state.selected() {
								Some(idx) => {
									if idx < self.msgs.len().saturating_sub(1) {
										self.scroll_state.select(Some(idx + 1));
									}
								},
								None => {
									if !self.msgs.is_empty() {
										self.scroll_state.select(Some(0))
									}
								},
							},
							crossterm::event::KeyCode::Char('k') => match self.scroll_state.selected() {
								Some(idx) => {
									if idx > 0 {
										self.scroll_state.select(Some(idx - 1));
									}
								},
								None => {
									if !self.msgs.is_empty() {
										self.scroll_state.select(Some(0))
									}
								},
							},
							_ => {},
						},
						_ => {},
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
				State::MsgListSelected => format!("In VIEW mode | Selected {:?} message", self.scroll_state.selected()),
			});
			f.render_widget(widget, help_area);

			let block = {
				let block = ratatui::widgets::Block::default().borders(ratatui::widgets::Borders::all()).title("Input");
				let block = if matches!(self.state, State::InputSelected) {
					block.border_style(ratatui::style::Style::default().fg(ratatui::style::Color::Yellow))
				} else {
					block
				};

				block
			};
			let widget = ratatui::widgets::Paragraph::new(self.input.clone()).block(block);
			f.render_widget(widget, input_area);

			let block = {
				let block = ratatui::widgets::Block::default().borders(ratatui::widgets::Borders::all()).title("Events");
				let block = if matches!(self.state, State::MsgListSelected) {
					block.border_style(ratatui::style::Style::default().fg(ratatui::style::Color::Yellow))
				} else {
					block
				};

				block
			};
			let msgs = self.msgs.iter().cloned().map(|msg| ratatui::widgets::ListItem::new(msg));
			let widget = ratatui::widgets::List::new(msgs)
				.block(block)
				.highlight_style(ratatui::style::Style::default().fg(ratatui::style::Color::Yellow));
			f.render_stateful_widget(widget, msgs_area, &mut self.scroll_state);
		})?;

		Ok(())
	}
}

impl<Req, Res, Err> Client<Req, Res, Err> {
	fn add_msg(&mut self, msg: String) {
		if self.msgs.len() >= MAX_MESSAGES {
			self.msgs.pop_front();
		}

		self.msgs.push_back(msg);
	}
}
