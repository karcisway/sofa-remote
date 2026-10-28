//! Optional WebRTC transport: the phone opens an unordered, no-retransmit data channel
//! (UDP underneath), so a lost Wi-Fi packet never stalls the inputs behind it the way TCP does.
//! Signalling (offer/answer) goes over the existing WebSocket.

use crate::session::Session;
use bytes::BytesMut;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use webrtc::data_channel::{DataChannel, DataChannelEvent};
use webrtc::peer_connection::{
    PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler, RTCConfigurationBuilder,
    RTCIceGatheringState, RTCSessionDescription,
};
use webrtc::runtime::{Runtime, Sender, channel, default_runtime};

pub type Peer = Arc<dyn PeerConnection>;

struct Handler {
    gather_done: Sender<()>,
    runtime: Arc<dyn Runtime>,
    session: Arc<Mutex<Session>>,
}

#[async_trait::async_trait]
impl PeerConnectionEventHandler for Handler {
    async fn on_ice_gathering_state_change(&self, state: RTCIceGatheringState) {
        if state == RTCIceGatheringState::Complete {
            let _ = self.gather_done.try_send(());
        }
    }

    async fn on_data_channel(&self, dc: Arc<dyn DataChannel>) {
        let session = self.session.clone();
        self.runtime.spawn(Box::pin(async move {
            while let Some(event) = dc.poll().await {
                match event {
                    DataChannelEvent::OnMessage(msg) => {
                        let reply = session.lock().unwrap().binary(&msg.data);
                        if let Some(reply) = reply {
                            let _ = dc.send(BytesMut::from(&reply[..])).await;
                        }
                    }
                    DataChannelEvent::OnClose => break,
                    _ => {}
                }
            }
            session.lock().unwrap().release_all();
        }));
    }
}

/// Answer the phone's offer. LAN only: no STUN/TURN, host candidates are enough.
pub async fn answer(session: Arc<Mutex<Session>>, offer_sdp: String) -> anyhow::Result<(String, Peer)> {
    let runtime = default_runtime().ok_or_else(|| anyhow::anyhow!("no async runtime for WebRTC"))?;
    let (gather_done, mut gather_rx) = channel::<()>(1);
    let handler = Arc::new(Handler { gather_done, runtime: runtime.clone(), session });
    let pc = PeerConnectionBuilder::new()
        .with_configuration(RTCConfigurationBuilder::new().build())
        .with_handler(handler)
        .with_runtime(runtime)
        .with_udp_addrs(vec!["0.0.0.0:0".to_string()])
        .build()
        .await?;
    pc.set_remote_description(RTCSessionDescription::offer(offer_sdp)?).await?;
    let answer = pc.create_answer(None).await?;
    pc.set_local_description(answer).await?;
    // No trickle ICE: wait until all candidates are in the answer
    let _ = tokio::time::timeout(Duration::from_secs(3), gather_rx.recv()).await;
    let sdp = pc.local_description().await.ok_or_else(|| anyhow::anyhow!("no local description"))?.sdp;
    Ok((sdp, Arc::new(pc)))
}

pub async fn close(peer: Peer) {
    let _ = peer.close().await;
}
