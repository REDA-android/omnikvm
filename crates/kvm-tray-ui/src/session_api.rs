//! HTTP handlers and E2E integration tests for [`crate::session::SessionManager`].

use crate::clip_sync::push_clipboard_over_quic;
use crate::codec::event_to_packet;
use crate::session::{
    post_start_client, post_start_host, release_modifiers, SessionManager,
};
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use kvm_input_hal::InputEvent;
use serde::Deserialize;
use serde_json::json;
use std::sync::atomic::Ordering;

#[derive(Deserialize)]
pub struct MoveReq {
    pub dx: Option<i32>,
    pub dy: Option<i32>,
}

pub async fn post_test_move(
    State(mgr): State<SessionManager>,
    Json(req): Json<MoveReq>,
) -> impl IntoResponse {
    let Some(c) = mgr.inner.lock().unwrap().client.clone() else {
        return Json(json!({ "ok": false, "error": "Aucune session client active" }));
    };
    let seq = c.seq.fetch_add(1, Ordering::Relaxed) as u32;
    let ev = InputEvent::MouseMove {
        dx: req.dx.unwrap_or(40),
        dy: req.dy.unwrap_or(0),
    };
    let Some(pkt) = event_to_packet(seq, ev) else {
        return Json(json!({ "ok": false, "error": "encode error" }));
    };
    let res = c.sealer.lock().unwrap().seal(&pkt.serialize());
    match c.conn.send_datagram(res.into()) {
        Ok(()) => {
            mgr.sent.fetch_add(1, Ordering::Relaxed);
            Json(json!({ "ok": true, "session": mgr.snapshot() }))
        }
        Err(e) => Json(json!({ "ok": false, "error": e.to_string() })),
    }
}

pub async fn post_toggle_remote(State(mgr): State<SessionManager>) -> impl IntoResponse {
    {
        let g = mgr.inner.lock().unwrap();
        let next = !g.hook_ctrl.is_remote_active();
        if !next {
            if let Some(c) = g.client.as_ref() {
                release_modifiers(c);
            }
        }
        g.hook_ctrl.set_remote_active(next);
    }
    Json(json!({ "ok": true, "session": mgr.snapshot() }))
}

#[derive(Deserialize)]
pub struct SendClipReq {
    pub text: Option<String>,
}

pub async fn post_send_clipboard(
    State(mgr): State<SessionManager>,
    Json(req): Json<SendClipReq>,
) -> impl IntoResponse {
    let target = {
        let g = mgr.inner.lock().unwrap();
        g.client
            .as_ref()
            .map(|c| (c.conn.clone(), c.session_key))
            .or_else(|| g.peer_conn.clone())
    };
    let Some((conn, sk)) = target else {
        return Json(json!({ "ok": false, "error": "Aucun pair connecté" }));
    };
    let text = req
        .text
        .filter(|t| !t.is_empty())
        .or_else(kvm_input_hal::win_clipboard::get_clipboard_text)
        .unwrap_or_default();
    if text.is_empty() {
        return Json(json!({ "ok": false, "error": "Presse-papiers vide" }));
    }
    let id = mgr.clips.fetch_add(1, Ordering::Relaxed) + 1;
    match push_clipboard_over_quic(&conn, &sk, id, &text).await {
        Ok(()) => Json(json!({ "ok": true, "bytes": text.len(), "session": mgr.snapshot() })),
        Err(e) => Json(json!({ "ok": false, "error": e })),
    }
}

async fn get_session(State(mgr): State<SessionManager>) -> impl IntoResponse {
    Json(mgr.snapshot())
}

async fn post_stop(State(mgr): State<SessionManager>) -> impl IntoResponse {
    mgr.stop();
    Json(json!({ "ok": true, "session": mgr.snapshot() }))
}

pub fn router(mgr: SessionManager) -> Router {
    Router::new()
        .route("/api/session", get(get_session))
        .route("/api/session/start-host", post(post_start_host))
        .route("/api/session/start-client", post(post_start_client))
        .route("/api/session/toggle-remote", post(post_toggle_remote))
        .route("/api/session/send-clipboard", post(post_send_clipboard))
        .route("/api/session/test-move", post(post_test_move))
        .route("/api/session/stop", post(post_stop))
        .with_state(mgr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{ClientReq, HostReq};
    use std::sync::atomic::{AtomicBool, AtomicU16};
    use std::sync::Arc;

    #[tokio::test]
    async fn live_ui_session_host_client_and_clipboard_e2e() {
        let dir = std::env::temp_dir().join(format!("omnikvm_ui_sess_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let host_mgr = SessionManager::new(
            Some(dir.join("host.json")),
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicU16::new(4433)),
        );
        let cli_mgr = SessionManager::new(
            Some(dir.join("cli.json")),
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicU16::new(4433)),
        );

        let _ = post_start_host(
            State(host_mgr.clone()),
            Json(HostReq {
                port: Some(0),
                pin: Some("654321".into()),
                mock: Some(true),
            }),
        )
        .await;
        let h_snap = host_mgr.snapshot();
        assert_eq!(h_snap.mode, "host");
        let port = h_snap.bind_addr.unwrap().split(':').next_back().unwrap().to_string();

        let _ = post_start_client(
            State(cli_mgr.clone()),
            Json(ClientReq {
                peer: format!("127.0.0.1:{port}"),
                pin: "654321".into(),
                edge: Some("right".into()),
                capture: Some(false),
            }),
        )
        .await;
        assert_eq!(cli_mgr.snapshot().mode, "client");

        let _ = post_test_move(
            State(cli_mgr.clone()),
            Json(MoveReq {
                dx: Some(25),
                dy: Some(-5),
            }),
        )
        .await;
        let _ = post_send_clipboard(
            State(cli_mgr.clone()),
            Json(SendClipReq {
                text: Some("Bonjour OmniKVM!".into()),
            }),
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(cli_mgr.snapshot().events_sent, 1);
        assert_eq!(host_mgr.snapshot().events_received, 1);
        assert_eq!(host_mgr.snapshot().clips_synced, 1);
        assert_eq!(
            host_mgr.last_clip.lock().unwrap().as_deref(),
            Some("Bonjour OmniKVM!")
        );

        cli_mgr.stop();
        host_mgr.stop();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
