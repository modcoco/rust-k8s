pub mod connector;
pub mod msg_handle;

use std::borrow::Cow;

use axum::{
    extract::{ws::WebSocket, WebSocketUpgrade},
    response::Response,
};
use common::{
    axum::{
        self,
        extract::{ws::Message, Query, RawPathParams},
        response::IntoResponse,
    },
    tokio::{self, sync::mpsc},
    tracing,
};
use connector::{pod_exec_connector, ContainerCoords, PodExecParams, PodExecUrl};
use kube::ServiceAccountToken;
use msg_handle::handle_websocket;
use serde::{Deserialize, Serialize};
use util::{err::AxumErr, rsp::Rsp};

pub async fn handler(ws: WebSocketUpgrade, raw_path_params: RawPathParams) -> Response {
    let coords = ContainerCoords::default().populate_from_raw_path_params(&raw_path_params);
    tracing::info!("{:?}", coords);

    let protocols: Vec<Cow<'static, str>> = vec![Cow::Borrowed("echo-protocol")];
    ws.protocols(protocols)
        .on_upgrade(|axum_socket| handle_socket(axum_socket, coords))
}

pub async fn handle_socket(mut axum_socket: WebSocket, coords: ContainerCoords) {
    let sat = ServiceAccountToken::new();

    let pod_exec_url = PodExecUrl::default().get_exec_url(&sat.kube_host, &sat.kube_port, &coords);
    let pod_exec_params = PodExecParams::default().get_pod_exec_params(&coords);

    let (tx_web, mut rx_web) = mpsc::channel::<Message>(100);
    let (tx_kube, mut rx_kube) = mpsc::channel(100);

    let conn = pod_exec_connector(&sat, &pod_exec_url, &pod_exec_params).await;
    match conn {
        Ok(mut kube_ws_stream) => {
            let mut closed = false;
            tokio::spawn(async move {
                handle_websocket(
                    &mut kube_ws_stream,
                    &mut rx_web,
                    &tx_kube,
                    &mut closed,
                    None,
                )
                .await;
            });
        }
        Err(err) => {
            tracing::error!("ERROR, {}", err)
        }
    };

    loop {
        tokio::select! {
            Some(client_msg) = axum_socket.recv() => {
                let client_msg = if let Ok(client_msg) = client_msg {
                    tracing::debug!("Received from client: {:?}", client_msg);
                    client_msg
                } else {
                    tracing::info!("Client disconnected, the msg isn't ok");
                    return;
                };

                if tx_web.send(client_msg).await.is_err() {
                    tracing::info!("Failed to send message to channel");
                }
            },
            Some(kube_msg) = rx_kube.recv() => {
                tracing::debug!("Received from kubernetes: {}", kube_msg);
                let kube_msg = Message::Text(kube_msg);
                if axum_socket.send(kube_msg).await.is_err() {
                    tracing::info!("Client disconnected, failed to send message");
                }
            }
        }
    }
}

#[derive(Default, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerReq {
    pub container: i32,
}

pub async fn container_list(Query(req): Query<ContainerReq>) -> Result<impl IntoResponse, AxumErr> {
    println!("{}", req.container);
    Ok(Rsp::success_with_optional_biz_status(
        vec![1, 2, 3],
        "Data fetched successfully.",
        Some(1),
    ))
}
