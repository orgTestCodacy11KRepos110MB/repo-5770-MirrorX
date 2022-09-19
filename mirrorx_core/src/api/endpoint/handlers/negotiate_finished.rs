use super::video_frame::serve_decoder;
use crate::{
    api::endpoint::{
        flutter_message::FlutterMediaMessage,
        message::{EndPointMessage, EndPointNegotiateFinishedRequest},
        ENDPOINTS, SEND_MESSAGE_TIMEOUT,
    },
    component::{
        desktop::{monitor::get_primary_monitor_params, Duplicator},
        video_encoder::Encoder,
    },
    core_error,
    error::{CoreError, CoreResult},
    utility::runtime::TOKIO_RUNTIME,
};
use flutter_rust_bridge::StreamSink;
use scopeguard::defer;
use tokio::sync::mpsc::Sender;

// static RESPONSE_CHANNELS: Lazy<
//     DashMap<(i64, i64), oneshot::Sender<EndPointNegotiateFinishedResponse>>,
// > = Lazy::new(|| DashMap::new());

pub struct NegotiateFinishedRequest {
    pub active_device_id: i64,
    pub passive_device_id: i64,
    // pub selected_monitor_id: String,
    pub expect_frame_rate: u8,
    pub texture_id: i64,
    pub video_texture_pointer: i64,
    pub update_frame_callback_pointer: i64,
}

pub async fn negotiate_finished(
    req: NegotiateFinishedRequest,
    stream: StreamSink<FlutterMediaMessage>,
) -> CoreResult<()> {
    let message_tx = ENDPOINTS
        .get(&(req.active_device_id, req.passive_device_id))
        .ok_or(core_error!("endpoint not exists"))?;

    let negotiate_req =
        EndPointMessage::NegotiateFinishedRequest(EndPointNegotiateFinishedRequest {
            // selected_monitor_id: req.selected_monitor_id,
            expected_frame_rate: req.expect_frame_rate,
        });

    // let (resp_tx, resp_rx) = tokio::sync::oneshot::channel();
    // RESPONSE_CHANNELS.insert(
    //     (
    //         req.active_device_id.to_owned(),
    //         req.passive_device_id.to_owned(),
    //     ),
    //     resp_tx,
    // );

    if let Err(err) = message_tx
        .send_timeout(negotiate_req, SEND_MESSAGE_TIMEOUT)
        .await
    {
        // RESPONSE_CHANNELS.remove(&(req.active_device_id, req.passive_device_id));
        return Err(core_error!(
            "negotiate_finished: message send failed ({})",
            err
        ));
    }

    // let _ = tokio::time::timeout(RECV_MESSAGE_TIMEOUT, resp_rx).await??;

    serve_decoder(req.active_device_id, req.passive_device_id, stream);

    Ok(())
}

pub async fn handle_negotiate_finished_request(
    active_device_id: i64,
    passive_device_id: i64,
    _: EndPointNegotiateFinishedRequest,
    message_tx: Sender<EndPointMessage>,
) {
    // todo: launch video and audio

    spawn_desktop_capture_and_encode_process(active_device_id, passive_device_id, message_tx);
}

#[cfg(target_os = "macos")]
fn spawn_desktop_capture_and_encode_process(
    active_device_id: i64,
    passive_device_id: i64,
    mut message_tx: Sender<EndPointMessage>,
) {
    let (capture_frame_tx, capture_frame_rx) = crossbeam::channel::bounded(180);

    TOKIO_RUNTIME.spawn_blocking(move || {
        defer! {
            tracing::info!(?active_device_id, ?passive_device_id, "desktop capture process exit");
        }

        let (monitor_id, monitor_height, monitor_width) = match get_primary_monitor_params() {
            Ok(params) => params,
            Err(err) => {
                tracing::error!(
                    ?active_device_id,
                    ?passive_device_id,
                    ?err,
                    "get_primary_monitor_params failed"
                );
                return;
            }
        };

        let mut encoder = match Encoder::new(monitor_width as i32, monitor_height as i32) {
            Ok(encoder) => encoder,
            Err(err) => {
                tracing::error!(
                    ?active_device_id,
                    ?passive_device_id,
                    ?err,
                    "initialize encoder failed"
                );
                return;
            }
        };

        let duplicator = match Duplicator::new(Some(monitor_id), capture_frame_tx) {
            Ok(duplicator) => duplicator,
            Err(err) => {
                tracing::error!(
                    ?active_device_id,
                    ?passive_device_id,
                    ?err,
                    "initialize encoder failed"
                );
                return;
            }
        };

        if let Err(err) = duplicator.start() {
            tracing::error!(
                ?active_device_id,
                ?passive_device_id,
                ?err,
                "desktop capture process start failed"
            );
            return;
        }

        defer! {
            let _ = duplicator.stop();
        }

        loop {
            match capture_frame_rx.recv() {
                Ok(capture_frame) => {
                    if message_tx.is_closed() {
                        tracing::error!(
                            ?active_device_id,
                            ?passive_device_id,
                            "message tx has closed, encode process will exit"
                        );
                        break;
                    }

                    if let Err(err) = encoder.encode(capture_frame, &mut message_tx) {
                        tracing::error!(
                            ?active_device_id,
                            ?passive_device_id,
                            ?err,
                            "video encode failed"
                        );
                        break;
                    }
                }
                Err(err) => {
                    tracing::error!(
                        ?active_device_id,
                        ?passive_device_id,
                        ?err,
                        "capture frame rx recv error"
                    );
                    break;
                }
            }
        }
    });
}

#[cfg(target_os = "windows")]
fn spawn_desktop_capture_and_encode_process(
    active_device_id: i64,
    passive_device_id: i64,
    mut message_tx: Sender<EndPointMessage>,
) {
    use crate::component::video_encoder::FFMPEGEncoderType;

    let (monitor_id, monitor_height, monitor_width) = match get_primary_monitor_params() {
        Ok(params) => params,
        Err(err) => {
            tracing::error!(
                ?active_device_id,
                ?passive_device_id,
                ?err,
                "get_primary_monitor_params failed"
            );
            return;
        }
    };

    let (capture_frame_tx, capture_frame_rx) = crossbeam::channel::bounded(180);

    TOKIO_RUNTIME.spawn_blocking(move || {
        defer! {
            tracing::info!(?active_device_id, ?passive_device_id, "desktop capture process exit");
        }

        let mut duplicator = match Duplicator::new(Some(monitor_id)) {
            Ok(duplicator) => duplicator,
            Err(err) => {
                tracing::error!(
                    ?active_device_id,
                    ?passive_device_id,
                    ?err,
                    "initialize encoder failed"
                );
                return;
            }
        };

        loop {
            match duplicator.capture() {
                Ok(capture_frame) => {
                    if let Err(err) = capture_frame_tx.try_send(capture_frame) {
                        if err.is_disconnected() {
                            tracing::error!("capture frame tx closed, capture process will exit");
                            break;
                        }
                    }
                }
                Err(err) => {
                    tracing::error!(
                        ?active_device_id,
                        ?passive_device_id,
                        ?err,
                        "capture frame failed"
                    );
                    break;
                }
            };
        }
    });

    TOKIO_RUNTIME.spawn_blocking(move || {
        defer! {
            tracing::info!(?active_device_id, ?passive_device_id, "encode process exit");
        }

        let mut encoder = match Encoder::new(
            FFMPEGEncoderType::Libx264,
            monitor_width as i32,
            monitor_height as i32,
        ) {
            Ok(encoder) => encoder,
            Err(err) => {
                tracing::error!(
                    ?active_device_id,
                    ?passive_device_id,
                    ?err,
                    "initialize encoder failed"
                );
                return;
            }
        };

        loop {
            match capture_frame_rx.recv() {
                Ok(capture_frame) => {
                    if message_tx.is_closed() {
                        tracing::error!(
                            ?active_device_id,
                            ?passive_device_id,
                            "message tx has closed, encode process will exit"
                        );
                        break;
                    }

                    if let Err(err) = encoder.encode(capture_frame, &mut message_tx) {
                        tracing::error!(
                            ?active_device_id,
                            ?passive_device_id,
                            ?err,
                            "video encode failed"
                        );
                        break;
                    }
                }
                Err(err) => {
                    tracing::error!(
                        ?active_device_id,
                        ?passive_device_id,
                        ?err,
                        "capture frame rx recv error"
                    );
                    break;
                }
            }
        }
    });
}
