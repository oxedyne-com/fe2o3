use crate::srv::{
    constant,
    context::ServerContext,
};

use oxedize_fe2o3_core::{
    prelude::*,
    rand::RanDef,
};
use oxedize_fe2o3_iop_crypto::enc::Encrypter;
use oxedize_fe2o3_iop_db::api::Database;
use oxedize_fe2o3_iop_hash::api::Hasher;
use oxedize_fe2o3_jdat::id::{
    IdDat,
    NumIdDat,
};
use oxedize_fe2o3_net::{
    conc::AsyncReadIterator,
    http::{
        handler::WebHandler,
        header::{
            HttpHeadline,
            HttpMethod,
        },
        msg::{
            HttpMessageReader,
            HttpMessage,
        },
        status::HttpStatus,
    },
    //smtp::handler::EmailHandler,
    ws::handler::WebSocketHandler,
};
use oxedize_fe2o3_syntax::SyntaxRef;

use std::{
    net::SocketAddr,
    pin::Pin,
};

use tokio::{
    net::TcpStream,
    io::AsyncWriteExt,
};
use tokio_rustls::server::TlsStream;


impl<
    const UIDL: usize,
    UID:    NumIdDat<UIDL> + 'static,
    ENC:    Encrypter + 'static,
    KH:     Hasher + 'static,
    DB:     Database<UIDL, UID, ENC, KH> + 'static, 
    //EH:     EmailHandler + 'static,
    WH:     WebHandler + 'static,
    WSH:    WebSocketHandler + 'static,
>
    //ServerContext<UIDL, UID, ENC, KH, DB, EH, WH, WSH>
    ServerContext<UIDL, UID, ENC, KH, DB, WH, WSH>
{
    pub async fn handle_https(
        self,
        mut stream: TlsStream<TcpStream>,
        handler:    WH,
        ws_handler: WSH,
        ws_syntax:  SyntaxRef,
        src_addr:   SocketAddr,
    )
        -> Outcome<()>
    {
        let id = fmt!("Https|Cx:{}", IdDat::<4, u32>::randef()); // Cx = Connection id.
    
        let (mut read_stream, mut write_stream) = tokio::io::split(&mut stream);
    
        let mut reader: HttpMessageReader<
            '_,
            { constant::HTTP_DEFAULT_HEADER_CHUNK_SIZE },
            { constant::HTTP_DEFAULT_BODY_CHUNK_SIZE },
            _,
        > = HttpMessageReader::new(Pin::new(&mut read_stream));

        let log_level = res!(self.cfg.log_level());
    
        loop {
            let result = reader.next().await;
            match result {
                Some(Ok(request)) => {
                    log!(log_level, "{}: Incoming from {:?}:", id, src_addr);
                    request.log(get_log_level!());
    
                    if request.is_websocket_upgrade() {
                        log!(log_level, "Connection upgrading to websocket...");
                        // Reunite the read and write streams before passing to the websocket handler.
                        let reunited_stream = read_stream.unsplit(write_stream);
                        return self.handle_websocket(
                            reunited_stream,
                            ws_handler,
                            ws_syntax,
                            request,
                            &id,
                        ).await;
                    }
    
                    let sid_opt = Self::get_session_id(&request, &src_addr);
    
                    let mut response = None;
                    let close_requested = request.get_connection_close(); // Close at end of request.
                    if close_requested {
                        let mut msg = HttpMessage::new_response(HttpStatus::OK);
                        msg.set_connection_close(true);
                        response = Some(msg);
                    }
    
                    match request.header.headline {
                        HttpHeadline::Request { method, loc } => {
                            let body = request.body;
                            match method {
                                HttpMethod::GET => {
                                    let result = handler.handle_get(
                                        loc,
                                        response,
                                        body,
                                        self.db.clone(),
                                        &sid_opt,
                                        &id,
                                    ).await;
                                    response = res!(result);
                                }
                                _ => fault!("{}: Unsupported HTTP request method '{}'.", id, method),
                            }
                        },
                        _ => fault!("{}: Unsupported HTTP '{:?}'.", id, request.header.headline),
                    }
    
                    log!(log_level, "Outgoing HTTPS message:");
                    match response {
                        Some(msg) => {
                            match msg.write_all(&mut write_stream).await {
                                Ok(()) => (),
                                Err(e) => return Err(err!(e,
                                    "{}: Could not send response.", id;
                                    IO, Network, Wire, Write)),
                            }
                        }
                        None => log!(log_level, " None"),
                    }
                }
                Some(Err(e)) => return Err(e),
                None => {
                    break;
                }
            }
        }
    
        // Gracefully close the TLS connection
        let reunited_stream = read_stream.unsplit(write_stream);
        let result = reunited_stream.shutdown().await;
        if let Err(e) = result {
            error!(e.into());
        }
    
        log!(log_level, "{}: Connection with {:?} closed.", id, src_addr);
    
        Ok(())
    }
}
