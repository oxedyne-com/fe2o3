use oxedize_fe2o3_core::{
    prelude::*,
};

use std::{
    convert::TryFrom,
};


#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum WebSocketStatusCode {
    NormalClosure,
    GoingAway,
    ProtocolError,
    UnsupportedData,
    NoStatusReceived,
    AbnormalClosure,
    InvalidFramePayloadData,
    PolicyViolation,
    MessageTooBig,
    MandatoryExtension,
    InternalServerError,
    ServiceRestart,
    TryAgainLater,
    TlsHandshake,
}

impl TryFrom<u16> for WebSocketStatusCode {
    type Error = Error<ErrTag>;

    fn try_from(value: u16) -> Outcome<Self> {
        Ok(match value {
            1000 => WebSocketStatusCode::NormalClosure,
            1001 => WebSocketStatusCode::GoingAway,
            1002 => WebSocketStatusCode::ProtocolError,
            1003 => WebSocketStatusCode::UnsupportedData,
            1005 => WebSocketStatusCode::NoStatusReceived,
            1006 => WebSocketStatusCode::AbnormalClosure,
            1007 => WebSocketStatusCode::InvalidFramePayloadData,
            1008 => WebSocketStatusCode::PolicyViolation,
            1009 => WebSocketStatusCode::MessageTooBig,
            1010 => WebSocketStatusCode::MandatoryExtension,
            1011 => WebSocketStatusCode::InternalServerError,
            1012 => WebSocketStatusCode::ServiceRestart,
            1013 => WebSocketStatusCode::TryAgainLater,
            1015 => WebSocketStatusCode::TlsHandshake,
            _ => return Err(err!(errmsg!(
                "Unrecognised websocket status code {}.", value,
            ), Conversion, Integer)),
        })
    }
}

impl From<WebSocketStatusCode> for u16 {
    fn from(status_code: WebSocketStatusCode) -> Self {
        match status_code {
            WebSocketStatusCode::NormalClosure              => 1000,
            WebSocketStatusCode::GoingAway                  => 1001,
            WebSocketStatusCode::ProtocolError              => 1002,
            WebSocketStatusCode::UnsupportedData            => 1003,
            WebSocketStatusCode::NoStatusReceived           => 1005,
            WebSocketStatusCode::AbnormalClosure            => 1006,
            WebSocketStatusCode::InvalidFramePayloadData    => 1007,
            WebSocketStatusCode::PolicyViolation            => 1008,
            WebSocketStatusCode::MessageTooBig              => 1009,
            WebSocketStatusCode::MandatoryExtension         => 1010,
            WebSocketStatusCode::InternalServerError        => 1011,
            WebSocketStatusCode::ServiceRestart             => 1012,
            WebSocketStatusCode::TryAgainLater              => 1013,
            WebSocketStatusCode::TlsHandshake               => 1015,
        }
    }
}

impl WebSocketStatusCode {
    pub fn to_bytes(&self) -> [u8; 2] {
        u16::from(*self).to_be_bytes()
    }
}
