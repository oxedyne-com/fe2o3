use oxedyne_fe2o3_core::{
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
    // Application defined
    Private(u16),   // 3000-4999; RFC 6455 §7.4.2 leaves the meaning to the application
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
            // An application's own code. A peer that closes with one is not
            // speaking a code this crate has to understand, and refusing to
            // decode the frame would turn the peer's goodbye into a read error
            // for a caller whose whole protocol lives in this range.
            3000..=4999 => WebSocketStatusCode::Private(value),
            _ => return Err(err!(
                "Unrecognised websocket status code {}.", value;
            Conversion, Integer)),
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
            WebSocketStatusCode::Private(code)              => code,
        }
    }
}

impl WebSocketStatusCode {
    pub fn to_bytes(&self) -> [u8; 2] {
        u16::from(*self).to_be_bytes()
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_an_application_code_survives_the_round_trip() -> Outcome<()> {
        for code in [3000u16, 4401, 4402, 4403, 4429, 4999] {
            let parsed = res!(WebSocketStatusCode::try_from(code));
            assert_eq!(parsed, WebSocketStatusCode::Private(code));
            assert_eq!(u16::from(parsed), code);
            assert_eq!(parsed.to_bytes(), code.to_be_bytes());
        }
        Ok(())
    }

    #[test]
    fn test_a_code_outside_the_application_range_is_still_refused() {
        // The gap below the application range, and the space above it. Widening
        // the range to "anything" would let a peer's typo through as a meaning.
        for code in [0u16, 999, 1004, 1014, 2999, 5000, 65_535] {
            assert!(WebSocketStatusCode::try_from(code).is_err(),
                "close code {} was accepted", code);
        }
    }
}
