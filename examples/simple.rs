#[tokio::main]
pub async fn main() -> color_eyre::Result<()> {
	let client: wire_cli::Client<(), (), ()> = wire_cli::Client::new(wire_cli::ClientCfg { url: "".into() });
	client.start().await
}
