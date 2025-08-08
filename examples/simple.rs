use futures::{SinkExt, StreamExt};

#[tokio::main]
pub async fn main() -> color_eyre::Result<()> {
    let url = "127.0.0.1:8080"; // Replace with your WebSocket URL
    let server_task = tokio::spawn(async move {
        let listener = tokio::net::TcpListener::bind(url).await.unwrap();
        while let Ok((stream, _)) = listener.accept().await {
            let mut stream = tokio_tungstenite::accept_async(stream).await.unwrap();
            while let Some(msg) = stream.next().await {
                match msg {
                    Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                        let res: Result<wire::TimestampedEvent<String>, String> =
                            Ok(wire::TimestampedEvent::new(text.to_string()));
                        #[cfg(feature = "out-json")]
                        let text = serde_json::to_string(&res).expect("request is always a string");
                        #[cfg(feature = "out-ron")]
                        let text = ron::to_string(&res).expect("request is always a string");
                        // Echo the message back
                        stream
                            .send(tokio_tungstenite::tungstenite::Message::Text(text.into()))
                            .await
                            .unwrap();
                    }
                    Err(_) => {}
                    _ => {}
                }
            }
        }
    });

    let client: wire_cli::Client<String, String, String> =
        wire_cli::Client::new(wire_cli::ClientCfg {
            url: format!("ws://{url}").into(),
        });
    let result = client.start().await;
    server_task.abort(); // Stop the server after the client finishes
    result
}
