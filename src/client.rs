use std::error::Error;

use futures_util::{SinkExt, StreamExt};
use json_ecs_sub::QuerySubReq;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::ServerMsg;

pub fn client() -> Result<(), Box<dyn Error>> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move {
        let mut stdin = InteractiveStdin::new();

        let (websocket, response) = connect_async("ws://localhost:3012/socket")
            .await
            .expect("Can't connect");
        println!("Connection response: {response:?}");
        let (mut write, mut read) = websocket.split();

        tokio::spawn(async move {
            while let Some(msg) = read.next().await {
                let msg = msg.unwrap();
                let string = msg.into_text().unwrap();
                let parsed: ServerMsg = serde_json::from_str(&string).unwrap();
                match parsed {
                    ServerMsg::Ack(x) => println!("Ack: {x:?}"),
                    ServerMsg::QuerySubResp(x) => {
                        println!("{}", serde_json::to_string(&x).unwrap())
                    }
                    ServerMsg::Text(x) => println!("Text: {x}"),
                }
            }
        });

        loop {
            println!("Enter query (e.g. Location Health) ");
            let line = stdin.next_line().await.unwrap().unwrap();
            let fetch = line.split(" ").map(|x| x.to_string()).collect();
            let msg = serde_json::to_string(&QuerySubReq {
                fetch,
                filter: vec![],
                id: "query_1".into(),
            })
            .unwrap();
            write.send(Message::text(msg)).await.unwrap();
            println!("Subscription message sent.");
        }
    });
    Ok(())
}

struct InteractiveStdin {
    chan: mpsc::Receiver<std::io::Result<String>>,
}

impl InteractiveStdin {
    fn new() -> Self {
        let (send, recv) = mpsc::channel(16);
        std::thread::spawn(move || {
            for line in std::io::stdin().lines() {
                if send.blocking_send(line).is_err() {
                    return;
                }
            }
        });
        InteractiveStdin { chan: recv }
    }

    /// Get the next line from stdin.
    ///
    /// Returns `Ok(None)` if stdin has been closed.
    ///
    /// This method is cancel safe.
    async fn next_line(&mut self) -> std::io::Result<Option<String>> {
        self.chan.recv().await.transpose()
    }
}
