//! The loopback plain-HTTP listener.
//!
//! `ServerConfig::loopback_http_port` binds 127.0.0.1 and ::1 and serves every vhost over plain
//! HTTP through `handle_https` in its `plain` mode, which takes the vhost from each request's Host
//! header and sends no HSTS. The bind goes through `bind_loopback`, which refuses any address that
//! is not loopback, so no config value can put an unencrypted listener on a public interface.

use crate::srv::{
    context::ServerContext,
    stop,
};

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_iop_crypto::enc::Encrypter;
use oxedyne_fe2o3_iop_db::api::Database;
use oxedyne_fe2o3_iop_hash::api::Hasher;
use oxedyne_fe2o3_jdat::id::NumIdDat;
use oxedyne_fe2o3_net::{
    http::handler::WebHandler,
    ws::handler::WebSocketHandler,
};

use std::{
    net::{
        IpAddr,
        Ipv4Addr,
        Ipv6Addr,
        SocketAddr,
    },
    sync::{
        atomic::{
            AtomicUsize,
            Ordering,
        },
        Arc,
    },
};

use tokio::net::TcpListener;

/// Binds `addr` only when it is a loopback address.
pub async fn bind_loopback(addr: SocketAddr) -> Outcome<TcpListener> {
    if !addr.ip().is_loopback() {
        return Err(err!(
            "The loopback HTTP listener was asked to bind {}, which is not a loopback address; \
            it serves plain HTTP and binds 127.0.0.1 and ::1 alone.", addr;
            Invalid, Input, Network));
    }
    Ok(res!(TcpListener::bind(&addr).await, IO, Network))
}

impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
    DB:     Database<UIDL, UID, ENC, KH> + 'static,
    WH:     WebHandler + 'static,
    WSH:    WebSocketHandler + 'static,
>
    ServerContext<UIDL, UID, ENC, KH, DB, WH, WSH>
{
    /// Serves plain HTTP on `port` at 127.0.0.1 and, where the host has IPv6, ::1. Each
    /// connection counts in `inflight`, the TLS port's count, so a stop waits for it and
    /// `max_conn` caps both together. Returns only when neither address could be bound.
    pub async fn run_loopback_http(
        self,
        port:       u16,
        inflight:   Arc<AtomicUsize>,
    )
        -> Outcome<()>
    {
        let mut listeners = Vec::new();
        for ip in [IpAddr::V4(Ipv4Addr::LOCALHOST), IpAddr::V6(Ipv6Addr::LOCALHOST)] {
            let addr = SocketAddr::new(ip, port);
            match bind_loopback(addr).await {
                Ok(l) => {
                    info!("Loopback HTTP listener bound to {}", addr);
                    listeners.push(l);
                }
                // A host without IPv6 has no ::1; 127.0.0.1 alone still serves.
                Err(e) => warn!("Loopback HTTP listener could not bind {}: {}", addr, e),
            }
        }
        if listeners.is_empty() {
            return Err(err!(
                "The loopback HTTP listener bound neither 127.0.0.1:{} nor [::1]:{}.", port, port;
                IO, Network, Init));
        }
        let mut tasks = Vec::new();
        for listener in listeners {
            tasks.push(tokio::spawn(self.clone().serve_loopback(listener, inflight.clone())));
        }
        for t in tasks {
            let _ = t.await;
        }
        Ok(())
    }

    /// Serves one bound listener; also what a test drives.
    pub async fn serve_loopback(
        self,
        listener:   TcpListener,
        inflight:   Arc<AtomicUsize>,
    ) {
        let max_conn = self.cfg.max_conn as usize;
        loop {
            let (stream, peer) = match listener.accept().await {
                Ok(pair) => pair,
                Err(e) => {
                    error!(err!(e, "Loopback HTTP listener: accept failed."; IO, Network));
                    continue;
                }
            };
            if max_conn != 0 && inflight.load(Ordering::Relaxed) >= max_conn {
                debug!("total-connection cap {} reached; dropping loopback TCP from {}.",
                    max_conn, peer);
                continue;
            }
            let counted = stop::InFlight::begin(&inflight);
            let ctx = self.clone();
            tokio::spawn(async move {
                let _counted = counted;
                if let Err(e) = ctx.handle_https(stream, None, peer, true).await {
                    error!(err!(e, "Error handling a loopback HTTP connection."; IO, Network));
                }
            });
        }
    }
}
