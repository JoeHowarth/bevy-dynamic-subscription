use std::{error::Error, sync::Arc, time::Duration};

use bevy::{app::ScheduleRunnerSettings, prelude::*};
use bevy_tokio_tasks::{TaskContext, TokioTasksPlugin, TokioTasksRuntime};
use clap::Parser;
use crossbeam_channel::{Receiver, Sender};
use futures_util::{
    stream::{SplitSink, SplitStream},
    SinkExt, StreamExt,
};
use json_ecs_sub::*;
use serde::{Deserialize, Serialize};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::Mutex,
};
use tokio_tungstenite::{
    tungstenite::{error::ProtocolError, Message},
    WebSocketStream,
};

pub mod client;

type Result<T = (), E = Box<dyn Error>> = core::result::Result<T, E>;

#[derive(Parser, Debug, Resource, Clone)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// It just works!
    #[clap(long, action)]
    pub is_server: bool,
}

#[derive(Debug, Serialize, Deserialize)]
enum ClientMsg {
    Subscribe(QuerySubReq),
    Unsubscribe(QueryId),
}

#[derive(Debug, Serialize, Deserialize)]
enum ServerMsg {
    Ack(QuerySubReq),
    QuerySubResp(QuerySubResp),
    Text(String),
}

fn main() -> Result {
    let args = Args::parse();
    if !args.is_server {
        client::client().unwrap();
    } else {
        server(args);
    }
    Ok(())
}

fn server(args: Args) {
    App::new()
        // .insert_resource(ScheduleRunnerSettings::run_once())
        .insert_resource(ScheduleRunnerSettings::run_loop(Duration::from_secs_f64(
            1.0,
        )))
        .insert_resource(args)
        .add_plugin(TokioTasksPlugin::default())
        .add_plugins(MinimalPlugins)
        .add_plugin(bevy::log::LogPlugin::default())
        .add_startup_system(setup)
        .add_system(query_runner)
        .add_system(spawner)
        .run();
}

#[derive(Debug, Component, Reflect, serde::Serialize)]
#[reflect(Serialize)]
struct Location {
    pub city: String,
}

#[derive(Debug, Component, Reflect, serde::Serialize)]
#[reflect(Serialize)]
struct Health {
    pub health: u32,
}

#[derive(Resource)]
struct DynamicQueryRequests(pub Receiver<QuerySubReq>);

#[derive(Resource)]
struct DynamicQueryResponse(pub Sender<QuerySubResp>);

fn setup(world: &mut World) {
    // let args = world.get_resource::<Args>().unwrap();
    let writes = SubscriptionWsWrites::default();
    world.insert_resource(writes.clone());
    world.insert_resource(EcsSubApi::default());
    world.register::<Health>();
    world.register::<Location>();
    let args = world.get_resource::<Args>().unwrap().clone();
    let rt = world.get_resource::<TokioTasksRuntime>().unwrap();
    rt.spawn_background_task(move |ctx| async move {
        println!("This print executes from a background Tokio runtime thread");
        if args.is_server {
            println!("Server");
            network(ctx, args, writes.clone()).await.unwrap();
        } else {
            println!("Client");
        }
    });
}

fn spawner(mut commands: Commands, mut i: Local<u64>) {
    match &*i {
        0 => {
            commands.spawn((
                Location {
                    city: "Salmon".into(),
                },
                Health { health: 99 },
            ));
        }
        1 => {
            commands.spawn((Location { city: "NYC".into() }, Health { health: 50 }));
        }
        2 => {
            commands.spawn((Location { city: "SLC".into() },));
        }
        3 => {
            commands.spawn(Health { health: 40 });
        }
        _ => {}
    };
    *i += 1;
}

fn query_runner(world: &mut World) {
    let api = world.get_resource::<EcsSubApi>().unwrap();
    let results = api.run_all_queries(world);

    for resp in results.iter() {
        println!("{}", serde_json::to_string_pretty(&resp).unwrap());
    }

    let Some(writes) = world.get_resource::<SubscriptionWsWrites>() else {
        debug!("SubscriptionWsWrites res not yet inserted");
        return;
    };
    let writes = writes.0.clone();

    let rt = world.get_resource::<TokioTasksRuntime>().unwrap();
    rt.spawn_background_task(move |_ctx| async move {
        let mut writes = writes.lock().await;

        let mut to_remove = Vec::new();
        for (i, writer) in writes.iter_mut().enumerate() {
            for result in &results {
                let msg = Message::text(
                    serde_json::to_string(&ServerMsg::QuerySubResp(result.clone())).unwrap(),
                );
                if let Err(e) = writer.send(msg.clone()).await {
                    match e {
                        tokio_tungstenite::tungstenite::Error::ConnectionClosed
                        | tokio_tungstenite::tungstenite::Error::Protocol(
                            ProtocolError::ResetWithoutClosingHandshake,
                        )
                        | tokio_tungstenite::tungstenite::Error::AlreadyClosed => {
                            error!("Caught: {}", e);
                            to_remove.push(i);
                            continue;
                        }
                        _ => error!("Uncaught {}", e),
                    }
                };
            }
        }
        for i in to_remove.iter().rev() {
            let _ = writes.remove(*i);
        }
    });
}

#[derive(Clone, Resource, Default)]
struct SubscriptionWsWrites(pub Arc<Mutex<Vec<SplitSink<WebSocketStream<TcpStream>, Message>>>>);

async fn network(
    ctx: TaskContext,
    _args: Args,
    subscription_ws_writes: SubscriptionWsWrites,
) -> Result {
    let addr = "127.0.0.1:3012".to_string();

    // Create the event loop and TCP listener we'll accept connections on.
    let listener = TcpListener::bind(&addr).await.expect("Failed to bind");
    info!("Listening on: {}", addr);

    while let Ok((stream, _)) = listener.accept().await {
        let addr = stream
            .peer_addr()
            .expect("connected streams should have a peer address");
        info!("Peer address: {}", addr);

        let ws_stream = tokio_tungstenite::accept_async(stream)
            .await
            .expect("Error during the websocket handshake occurred");
        let (write, read) = ws_stream.split();

        subscription_ws_writes.0.lock().await.push(write);

        info!("New WebSocket connection: {}", addr);
        tokio::spawn(incoming(ctx.clone(), read));
    }

    Ok(())
}

async fn incoming(mut ctx: TaskContext, mut read: SplitStream<WebSocketStream<TcpStream>>) {
    while let Some(msg) = read.next().await {
        let Ok(msg) = msg else {
            error!("{:?}", msg);
            continue;
        };
        let Message::Text(msg) = msg else {
                            println!("Expected only test messages");
                            continue;
                        };
        let query: QuerySubReq = serde_json::from_str(&msg).unwrap();
        println!("Req: {query:?}");
        ctx.run_on_main_thread(|ctx| {
            let api = ctx.world.remove_resource::<EcsSubApi>().unwrap_or_default();
            api.subscribe_components(query, ctx.world);
            ctx.world.insert_resource(api);
        })
        .await;
        println!("Inserted subscription request into EcsSubApi within world");
    }
}
